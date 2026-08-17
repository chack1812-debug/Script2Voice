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

/// 実行中の一括実行タスク一式。中断時に生成本体を future ごと落とすために保持する。
struct RunTask {
    /// 生成本体。`abort` すると future が drop され、`GenerationLock` の Drop で
    /// 出力ロック `.s2v_generation*.lock` が解放される。
    work: tokio::task::AbortHandle,
    /// 本体の完了を待って後始末(busy 解除・完了通知)を行う見張りタスク。
    /// 本体が drop され切ってからでないと完了しないため、これを待てばロック解放も保証される。
    watcher: tokio::task::JoinHandle<()>,
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
    /// 直近に起動した一括実行。中断(GUI終了・キャンセル)の後始末に使う。
    run_task: std::sync::Mutex<Option<RunTask>>,
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
            run_task: std::sync::Mutex::new(None),
        })
    }

    /// GUI 終了時に呼ぶ(実行中の生成を打ち切り、自動起動したエンジンを停止)。
    pub fn shutdown(&self) {
        self.abort_run();
        self.engines.shutdown_all();
    }

    /// 「⏹ キャンセル」から呼ぶ。実行中の一括実行を中断する。
    ///
    /// フラグを立てるだけでは、既に走り出した合成のHTTP応答待ち(既定180秒)が明けるまで
    /// 出力ロックを握ったまま・busy のままになる。本体を abort して future ごと落とし、
    /// `GenerationLock` の Drop を即座に走らせる。
    /// UI スレッドを固めないよう完了は待たない（後始末は見張りタスクが行う）。
    pub fn cancel_run(&self) {
        self.cancel.store(true, Ordering::SeqCst);
        if let Some(task) = self.run_task.lock().unwrap().as_ref() {
            task.work.abort();
        }
    }

    /// 実行中の一括実行を打ち切り、生成が確保している出力ロックの解放まで待つ。
    ///
    /// 待たずに戻ると、GUI 終了時はそのままプロセスが落ちて
    /// `.s2v_generation*.lock` が出力フォルダに残る。
    fn abort_run(&self) {
        let Some(task) = self.run_task.lock().unwrap().take() else {
            return;
        };
        self.cancel.store(true, Ordering::SeqCst);
        task.work.abort();
        let _ = self.rt.block_on(task.watcher);
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
        let (tx, progress_tx, engines, activated, config, cancel, busy) = (
            self.tx.clone(),
            self.tx.clone(), // 進捗中継スレッドへ渡す分（本体タスクへ move する）
            Arc::clone(&self.engines),
            Arc::clone(&self.activated),
            Arc::clone(&self.config),
            Arc::clone(&self.cancel),
            Arc::clone(&self.busy_run),
        );
        // 生成本体は独立したタスクにする。中断時に abort して future ごと落とすことで、
        // 保持中の RAII ガード(出力ロック)の Drop を確実に走らせる。
        let work = self.rt.spawn(async move {
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
                let fwd = progress_tx;
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
            res.map_err(|e| format!("{e:#}"))
        });

        // 本体の完了(正常・失敗・中断のいずれでも)を待って後始末する見張り。
        let abort = work.abort_handle();
        let watcher = self.rt.spawn(async move {
            let result = match work.await {
                Ok(res) => res,
                Err(e) if e.is_cancelled() => Err("中断しました".to_string()),
                Err(e) => Err(format!("内部エラー: {e}")),
            };
            busy.store(false, Ordering::SeqCst);
            let _ = tx.send(JobMsg::RunFinished { result });
        });
        *self.run_task.lock().unwrap() = Some(RunTask { work: abort, watcher });
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::time::{Duration, Instant};

    const TEST_SCRIPT: &str = r#"@cast
めたん:四国めたん:ノーマル,voicevox,pan=0,distance=1.0,volume=1.0

@scene 01_テスト
@script
めたん:こんにちは。
めたん:さようなら。
"#;

    /// テスト用の偽 TTS エンジン。`/version` `/speakers` `/audio_query` には即答するが、
    /// `/synthesis` には応答を返さず接続を握り続ける（=「合成中で止まっている」状態を作る）。
    /// リスナーはテストプロセスが終わるまで生きていればよいのでリークさせる。
    fn spawn_stalling_engine() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                std::thread::spawn(move || serve(stream));
            }
        });
        port
    }

    fn serve(mut stream: TcpStream) {
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        loop {
            let mut request_line = String::new();
            if reader.read_line(&mut request_line).unwrap_or(0) == 0 {
                return;
            }
            let mut content_length = 0usize;
            loop {
                let mut header = String::new();
                if reader.read_line(&mut header).unwrap_or(0) == 0 {
                    return;
                }
                if header.trim().is_empty() {
                    break;
                }
                let lower = header.to_ascii_lowercase();
                if let Some(v) = lower.strip_prefix("content-length:") {
                    content_length = v.trim().parse().unwrap_or(0);
                }
            }
            if content_length > 0 {
                let mut body = vec![0u8; content_length];
                if reader.read_exact(&mut body).is_err() {
                    return;
                }
            }
            let path = request_line.split_whitespace().nth(1).unwrap_or("");
            let payload = if path.starts_with("/version") {
                "\"0.0.0-test\"".to_string()
            } else if path.starts_with("/speakers") {
                r#"[{"name":"四国めたん","styles":[{"name":"ノーマル","id":2}]}]"#.to_string()
            } else if path.starts_with("/audio_query") {
                r#"{"speedScale":1.0}"#.to_string()
            } else {
                // /synthesis: 応答せず握り続ける。テストが終わればプロセスごと消える。
                std::thread::sleep(Duration::from_secs(600));
                return;
            };
            let res = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{payload}",
                payload.len()
            );
            if stream.write_all(res.as_bytes()).is_err() {
                return;
            }
            let _ = stream.flush();
        }
    }

    fn test_config(port: u16) -> Config {
        // aivis/xtts は使わないので到達しないポートを割り当てる
        let toml = format!(
            r#"
[voicevox]
url = "http://127.0.0.1:{port}"
[aivis]
url = "http://127.0.0.1:1"
[xtts]
url = "http://127.0.0.1:2"

[audio]
sample_rate = 24000
microphone_spacing = 0.2
sound_speed = 340.0
air_absorption_coeff = 0.05
room_size = 0.1
reverb_wet = 0.3
reference_dist = 1.0
reference_gain_db = -5.0
max_gain_db = -1.0
mic_directivity = 0.5
mic_angle = 45.0

[audio.engine_volume_offsets]
voicevox = 1.0
aivis = 1.0
xtts = 1.0

[concurrency]
voicevox = 2
aivis = 2
xtts = 2
audio_process = 2

[bgm]
crossfade_s = 1.0
se_fade_out_s = 0.05
"#
        );
        Config::from_toml(&toml).unwrap()
    }

    fn wait_until(cond: impl Fn() -> bool, timeout: Duration) -> bool {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if cond() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        cond()
    }

    /// 一括実行の途中で GUI を閉じた（= `App::on_exit` → `Jobs::shutdown`）ときに、
    /// 生成中に確保している出力ロック `.s2v_generation*.lock` が解放されること。
    ///
    /// 残したままだと出力フォルダにロックの残骸が積み、次回以降の生成が
    /// 連番 `_1`,`_2`... へフォールバックしかねない。
    #[test]
    fn shutdown_during_run_releases_generation_lock() {
        let port = spawn_stalling_engine();
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("台本.txt");
        std::fs::write(&script, TEST_SCRIPT).unwrap();

        let jobs = Jobs::new(test_config(port)).unwrap();
        jobs.run_all(script.clone());

        let lock = dir.path().join("台本").join(".s2v_generation.lock");
        assert!(
            wait_until(|| lock.exists(), Duration::from_secs(30)),
            "前提: 合成中で止まり、出力ロックを確保しているはず"
        );

        jobs.shutdown();

        assert!(
            !lock.exists(),
            "GUI終了の後始末で出力ロックが解放されるべき: {}",
            lock.display()
        );
    }

    /// 「⏹ キャンセル」を押したら、合成がエンジンの応答待ちで固まっていても
    /// 速やかに実行が終わり、出力ロックが解放され、再実行できる状態に戻ること。
    ///
    /// 応答待ちの完了を待ってから畳んでいると、HTTPのリクエストタイムアウト(既定180秒)まで
    /// ロックを握ったまま・busy のままになり、実質「キャンセルが効かない」。
    #[test]
    fn cancel_run_releases_generation_lock_promptly() {
        let port = spawn_stalling_engine();
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("台本.txt");
        std::fs::write(&script, TEST_SCRIPT).unwrap();

        let jobs = Jobs::new(test_config(port)).unwrap();
        jobs.run_all(script.clone());

        let lock = dir.path().join("台本").join(".s2v_generation.lock");
        assert!(
            wait_until(|| lock.exists(), Duration::from_secs(30)),
            "前提: 合成中で止まり、出力ロックを確保しているはず"
        );

        jobs.cancel_run();

        assert!(
            wait_until(|| !lock.exists(), Duration::from_secs(5)),
            "キャンセル後すぐに出力ロックが解放されるべき: {}",
            lock.display()
        );
        assert!(
            wait_until(|| !jobs.busy_run.load(Ordering::SeqCst), Duration::from_secs(5)),
            "キャンセル後は再実行できるよう busy_run が下りるべき"
        );
    }
}
