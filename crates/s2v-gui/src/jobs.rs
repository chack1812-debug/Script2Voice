use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

use s2v_audio::AudioProcessor;
use s2v_core::{Config, ScriptParser};
use s2v_engines::EngineManager;
use script2voice::{build_engine_manager, ProduceEvent, Producer};

use crate::scene_line::LabParams;
use crate::script_model::PreviewLine;

/// バックグラウンドジョブ → UI への通知。
pub enum JobMsg {
    PreviewReady { line_no: usize, wav: PathBuf, raw: PathBuf },
    PreviewFailed { line_no: usize, error: String },
    RunPhase(String),
    RunProgress { done: usize, total: usize },
    RunFinished { result: Result<PathBuf, String> },
    LabReady { wav: PathBuf, params: LabParams },
    LabFailed { error: String },
}

pub struct Jobs {
    rt: tokio::runtime::Runtime,
    tx: Sender<JobMsg>,
    pub rx: Receiver<JobMsg>,
    config: Arc<Config>,
    engines: Arc<EngineManager>,
    processor: Arc<AudioProcessor>,
    activated: Arc<tokio::sync::Mutex<HashSet<String>>>,
    pub cancel: Arc<AtomicBool>,
    pub busy_run: Arc<AtomicBool>,
    pub busy_preview: Arc<AtomicBool>,
    pub busy_lab: Arc<AtomicBool>,
    /// プレビュー・ラボ出力の一時 WAV 置き場。Jobs の drop でディレクトリごと削除されるため、
    /// 履歴(History)等がここのファイルを参照する場合は Jobs より先に手放すこと。
    tmp: tempfile::TempDir,
    lab_seq: std::sync::atomic::AtomicUsize,
    preview_seq: std::sync::atomic::AtomicUsize,
}

impl Jobs {
    pub fn new(config: Config) -> anyhow::Result<Self> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()?;
        let engines = Arc::new(build_engine_manager(&config));
        let processor = Arc::new(AudioProcessor::new(config.audio.clone()));
        let (tx, rx) = std::sync::mpsc::channel();
        Ok(Self {
            rt,
            tx,
            rx,
            config: Arc::new(config),
            engines,
            processor,
            activated: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
            cancel: Arc::new(AtomicBool::new(false)),
            busy_run: Arc::new(AtomicBool::new(false)),
            busy_preview: Arc::new(AtomicBool::new(false)),
            busy_lab: Arc::new(AtomicBool::new(false)),
            tmp: tempfile::tempdir()?,
            lab_seq: std::sync::atomic::AtomicUsize::new(0),
            preview_seq: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    /// GUI 終了時に呼ぶ(自動起動したエンジンを停止)。
    pub fn shutdown(&self) {
        self.engines.shutdown_all();
    }

    /// 未起動ならエンジンを起動する(preview/run の前段で共用)。
    async fn ensure_engines(
        engines: &Arc<EngineManager>,
        activated: &Arc<tokio::sync::Mutex<HashSet<String>>>,
        required: HashSet<String>,
    ) -> anyhow::Result<()> {
        let mut set = activated.lock().await;
        let missing: HashSet<String> = required.difference(&set).cloned().collect();
        if !missing.is_empty() {
            engines.activate_required(&missing).await?;
            set.extend(missing);
        }
        Ok(())
    }

    /// 台本1行のプレビュー(合成＋音響処理)。完了は JobMsg::PreviewReady。
    pub fn preview(&self, line: PreviewLine) {
        if self.busy_preview.swap(true, Ordering::SeqCst) {
            return; // 実行中は無視(UI 側でもボタン無効化)
        }
        let (tx, engines, processor, activated, busy) = (
            self.tx.clone(),
            Arc::clone(&self.engines),
            Arc::clone(&self.processor),
            Arc::clone(&self.activated),
            Arc::clone(&self.busy_preview),
        );
        let seq = self.preview_seq.fetch_add(1, Ordering::SeqCst);
        let raw = self.tmp.path().join(format!("preview_{:04}_{seq}_raw.wav", line.no));
        let out = self.tmp.path().join(format!("preview_{:04}_{seq}.wav", line.no));
        self.rt.spawn(async move {
            tracing::info!("試聴: 行{} の合成を開始します", line.no);
            let res: anyhow::Result<()> = async {
                let mut req = HashSet::new();
                req.insert(line.cast.engine_type.clone());
                Self::ensure_engines(&engines, &activated, req).await?;
                engines.synthesize(&line.text, &line.cast, &raw).await?;
                tracing::info!("試聴: 行{} 合成完了、音響処理中", line.no);
                let (p, r, o, c, s) = (
                    Arc::clone(&processor), raw.clone(), out.clone(),
                    line.cast.clone(), line.scene_config.clone(),
                );
                tokio::task::spawn_blocking(move || p.process(&r, &o, &c, &s)).await??;
                Ok(())
            }
            .await;
            busy.store(false, Ordering::SeqCst);
            let _ = match res {
                Ok(()) => {
                    tracing::info!("試聴: 行{} 準備完了", line.no);
                    tx.send(JobMsg::PreviewReady { line_no: line.no, wav: out, raw })
                }
                Err(e) => {
                    tracing::error!("試聴失敗: 行{} {e:#}", line.no);
                    tx.send(JobMsg::PreviewFailed { line_no: line.no, error: format!("{e:#}") })
                }
            };
        });
    }

    /// 一括実行(CLI と同一出力)。進捗は RunPhase / RunProgress、完了は RunFinished。
    pub fn run_all(&self, script_path: PathBuf) {
        if self.busy_run.swap(true, Ordering::SeqCst) {
            return;
        }
        self.cancel.store(false, Ordering::SeqCst);
        let (tx, engines, activated, config, cancel, busy) = (
            self.tx.clone(),
            Arc::clone(&self.engines),
            Arc::clone(&self.activated),
            Arc::clone(&self.config),
            Arc::clone(&self.cancel),
            Arc::clone(&self.busy_run),
        );
        self.rt.spawn(async move {
            let res: anyhow::Result<PathBuf> = async {
                let text = std::fs::read_to_string(&script_path)?;
                let mut parser = ScriptParser::new();
                let scenes = parser.parse_str(crate::script_model::strip_bom(&text))?;
                let project_name = script_path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .ok_or_else(|| anyhow::anyhow!("台本ファイル名が不正です"))?;
                let project_dir = script_path
                    .parent()
                    .unwrap_or_else(|| Path::new("."))
                    .join(project_name);
                std::fs::create_dir_all(&project_dir)?;

                let mut required = HashSet::new();
                for sc in &scenes {
                    for c in sc.casts.values() {
                        required.insert(c.engine_type.clone());
                    }
                }
                Self::ensure_engines(&engines, &activated, required).await?;

                // ProduceEvent → JobMsg 変換(std mpsc を中継)
                let (ev_tx, ev_rx) = std::sync::mpsc::channel::<ProduceEvent>();
                let fwd = tx.clone();
                std::thread::spawn(move || {
                    for ev in ev_rx {
                        let _ = match ev {
                            ProduceEvent::Phase(p) => fwd.send(JobMsg::RunPhase(p)),
                            ProduceEvent::ItemFinished { done, total } => {
                                fwd.send(JobMsg::RunProgress { done, total })
                            }
                            ProduceEvent::Finished => Ok(()),
                        };
                    }
                });

                let producer = Producer::new(Arc::clone(&engines), &config, &project_dir)?;
                producer.produce_with_events(&scenes, Some(ev_tx), Some(cancel)).await?;
                Ok(project_dir)
            }
            .await;
            busy.store(false, Ordering::SeqCst);
            let _ = tx.send(JobMsg::RunFinished { result: res.map_err(|e| format!("{e:#}")) });
        });
    }

    /// 音響ラボ: 入力 WAV(任意 WAV or 行プレビューの raw)に音響処理を適用。
    pub fn lab_process(&self, input: PathBuf, params: LabParams) {
        if self.busy_lab.swap(true, Ordering::SeqCst) {
            return;
        }
        let seq = self.lab_seq.fetch_add(1, Ordering::SeqCst);
        let out = self.tmp.path().join(format!("lab_{seq:04}.wav"));
        let (tx, processor, busy) = (
            self.tx.clone(),
            Arc::clone(&self.processor),
            Arc::clone(&self.busy_lab),
        );
        let cast = params.to_cast();
        let scene = params.to_scene_config("ラボ");
        self.rt.spawn(async move {
            let res = tokio::task::spawn_blocking(move || {
                processor.process(&input, &out, &cast, &scene).map(|_| out)
            })
            .await;
            busy.store(false, Ordering::SeqCst);
            let _ = match res {
                Ok(Ok(out)) => tx.send(JobMsg::LabReady { wav: out, params }),
                Ok(Err(e)) => tx.send(JobMsg::LabFailed { error: format!("{e:#}") }),
                Err(e) => tx.send(JobMsg::LabFailed { error: format!("内部エラー: {e}") }),
            };
        });
    }
}
