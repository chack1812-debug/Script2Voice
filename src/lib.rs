use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use reqwest::Client;
use s2v_audio::AudioProcessor;
use s2v_core::{BgmConfig, Cast, Config, Scene, ScriptCommand, ScriptItem, TimelineProcessor};
use s2v_engines::{EngineManager, HttpEngine, XttsEngine};
use s2v_export::Exporter;
use tokio::sync::Semaphore;
use tracing::{info, warn, error};

/// `--config` 省略時に使用する設定ファイルパスを決定する。
/// 明示指定があればそれを優先し、なければ実行ファイルと同じディレクトリの `config.toml` を返す。
pub fn resolve_config_path(explicit: Option<PathBuf>, exe_path: Option<&std::path::Path>) -> PathBuf {
    if let Some(path) = explicit {
        return path;
    }
    exe_path
        .and_then(|p| p.parent())
        .map(|dir| dir.join("config.toml"))
        .unwrap_or_else(|| PathBuf::from("config.toml"))
}

/// HTTPエンジン呼び出し用の共有 `Client` を、設定された connect/request タイムアウトで構築する。
/// タイムアウト未設定のままだと、エンジンが接続だけ受け付けて応答しない場合に
/// 1台詞の処理が無期限に待機してしまう。
fn build_http_client(http: &s2v_core::HttpConfig) -> Client {
    Client::builder()
        .connect_timeout(std::time::Duration::from_secs(http.connect_timeout_s))
        .timeout(std::time::Duration::from_secs(http.request_timeout_s))
        .build()
        .expect("reqwest Client の構築に失敗しました")
}

/// config から3エンジン（voicevox/aivis/xtts）を登録した EngineManager を構築する。
pub fn build_engine_manager(config: &Config) -> EngineManager {
    use std::time::Duration;
    let timeout = |secs: Option<u64>| Duration::from_secs(secs.unwrap_or(60));
    let client = Arc::new(build_http_client(&config.http));
    let mut em = EngineManager::new();
    em.register(
        "voicevox",
        Arc::new(HttpEngine::with_exe_path(
            "voicevox", &config.voicevox.url, Arc::clone(&client), config.voicevox.exe_path.clone(),
        ).with_args(config.voicevox.args.clone())
         .with_startup_timeout(timeout(config.voicevox.startup_timeout_s))),
    );
    em.register(
        "aivis",
        Arc::new(HttpEngine::with_exe_path(
            "aivis", &config.aivis.url, Arc::clone(&client), config.aivis.exe_path.clone(),
        ).with_args(config.aivis.args.clone())
         .with_startup_timeout(timeout(config.aivis.startup_timeout_s))),
    );
    em.register(
        "xtts",
        Arc::new(XttsEngine::with_exe_path(
            "xtts", &config.xtts.url, Arc::clone(&client), config.xtts.exe_path.clone(),
        ).with_args(config.xtts.args.clone())
         .with_startup_timeout(timeout(config.xtts.startup_timeout_s))),
    );
    em
}

/// produce_with_events が送出する進捗イベント。
#[derive(Debug, Clone)]
pub enum ProduceEvent {
    /// フェーズの開始（"準備" / "合成" / "タイムライン" / "書き出し"）
    Phase(String),
    /// 1行の合成＋音響処理が完了
    ItemFinished { done: usize, total: usize },
    /// 全処理完了
    Finished,
}

fn emit(events: &Option<Sender<ProduceEvent>>, ev: ProduceEvent) {
    if let Some(tx) = events {
        let _ = tx.send(ev); // 受信側が閉じていても処理は続行
    }
}

fn is_cancelled(cancel: &Option<Arc<AtomicBool>>) -> bool {
    cancel.as_ref().map(|c| c.load(Ordering::SeqCst)).unwrap_or(false)
}

pub struct Producer {
    engine_manager: Arc<EngineManager>,
    audio_processor: Arc<AudioProcessor>,
    audio_dir: PathBuf,
    project_root: PathBuf,
    concurrency: ConcurrencyConfig,
    bgm_config: BgmConfig,
    sample_rate: u32,
    fcpxml_fps: s2v_export::FrameRate,
}

/// 話者交代時のポーズ判定 (Python版 producer.py:188-193 相当)。
/// `None` を返した場合は `advance_after_speech` の既定値 (sentence pause) が使われる。
fn speech_pause(last_cast: Option<&str>, cast_name: &str, cast_pause_ms: f64) -> Option<f64> {
    match last_cast {
        Some(name) if name != cast_name => Some(cast_pause_ms),
        _ => None, // sentence pause (last_cast == None or same cast as previous)
    }
}

struct ConcurrencyConfig {
    voicevox: usize,
    aivis: usize,
    xtts: usize,
    audio_process: usize,
}

struct SynthTask {
    cast: Cast,
    text: String,
    display_text: String,
    raw_path: PathBuf,
    final_path: PathBuf,
    scene_config: s2v_core::SceneConfig,
    duration_ms: f64,
}

/// 合成または音響処理に失敗した台詞の記録（タイムラインには登録しない）。
struct TaskFailure {
    text: String,
    cast_name: String,
    reason: String,
}

impl Producer {
    pub fn new(
        engine_manager: Arc<EngineManager>,
        config: &Config,
        project_root: impl Into<PathBuf>,
    ) -> anyhow::Result<Self> {
        let project_root = project_root.into();
        let audio_dir = project_root.join("audio");
        std::fs::create_dir_all(&audio_dir)?;
        std::fs::create_dir_all(project_root.join("timeline"))?;

        let cpu_cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
        let audio_concurrency = if config.concurrency.audio_process == 0 {
            cpu_cores
        } else {
            config.concurrency.audio_process
        };

        Ok(Self {
            engine_manager,
            audio_processor: Arc::new(AudioProcessor::new(config.audio.clone())),
            audio_dir,
            project_root,
            concurrency: ConcurrencyConfig {
                voicevox: config.concurrency.voicevox,
                aivis: config.concurrency.aivis,
                xtts: config.concurrency.xtts,
                audio_process: audio_concurrency,
            },
            bgm_config: config.bgm.clone(),
            sample_rate: config.audio.sample_rate,
            fcpxml_fps: if config.export.fcpxml_2997fps {
                s2v_export::FrameRate::Fps2997
            } else {
                s2v_export::FrameRate::Fps30
            },
        })
    }

    pub async fn produce(&self, scenes: &[Scene]) -> anyhow::Result<()> {
        self.produce_with_events(scenes, None, None).await
    }

    pub async fn produce_with_events(
        &self,
        scenes: &[Scene],
        events: Option<Sender<ProduceEvent>>,
        cancel: Option<Arc<AtomicBool>>,
    ) -> anyhow::Result<()> {
        emit(&events, ProduceEvent::Phase("準備".into()));
        // ── Phase 1: パス割り当て ─────────────────────────────────────────
        let mut tasks: Vec<(usize, usize, SynthTask)> = Vec::new(); // (scene_idx, item_idx, task)
        let mut counter = 1usize;

        for (si, scene) in scenes.iter().enumerate() {
            for (ii, item) in scene.items.iter().enumerate() {
                let ScriptItem::Speech { cast_name, text, display_text, offset_params, scene_config } = item else {
                    continue;
                };
                let Some(cast) = scene.casts.get(cast_name) else {
                    warn!("キャスト '{cast_name}' が未定義です。スキップします。");
                    continue;
                };
                let effective = cast.with_offsets(offset_params);
                let filename = format!("voice_{counter:04}.wav");
                counter += 1;
                let raw_path = self.audio_dir.join(filename.replace(".wav", "_raw.wav"));
                let final_path = self.audio_dir.join(&filename);
                tasks.push((si, ii, SynthTask {
                    cast: effective,
                    text: text.clone(),
                    display_text: display_text.clone(),
                    raw_path,
                    final_path,
                    scene_config: scene_config.clone(),
                    duration_ms: 0.0,
                }));
            }
        }
        info!("Phase1完了: {} 件の speech アイテムを登録しました。", tasks.len());

        let total = tasks.len();
        let done = Arc::new(AtomicUsize::new(0));
        emit(&events, ProduceEvent::Phase("合成".into()));

        // ── 出力ロック対策: 生成一式の共通連番サフィックスを決定 ───────────
        let default_files: Vec<PathBuf> = tasks.iter()
            .map(|(_, _, t)| t.final_path.clone())
            .chain([
                self.project_root.join("timeline").join("subtitles.srt"),
                self.project_root.join("timeline").join("timeline.fcpxml"),
                self.project_root.join("full_dialogue.wav"),
            ])
            .collect();
        // ロックファイルにより、他プロセスが同じ台本を同時処理していても同じsuffixを
        // 選ばないようにする(TOCTOU対策)。生成完了までこのガードを保持し続ける。
        let (suffix, _generation_lock) = s2v_export::resolve_generation_suffix(&default_files, &self.project_root, 100)?;
        if !suffix.is_empty() {
            warn!("出力ファイルのいずれかが使用中のため、今回の生成一式を連番 {suffix} で保存します。");
        }
        for (_, _, t) in tasks.iter_mut() {
            t.final_path = s2v_export::with_suffix(&t.final_path, &suffix);
        }

        // ── Phase 2: 並列合成 + 音響処理 ──────────────────────────────────
        // IRキャッシュ事前ウォームアップ
        let reverb_params: Vec<(f64, usize)> = tasks.iter()
            .map(|(_, _, t)| {
                let rs = t.cast.params.get("room_size").and_then(|v| v.as_f64())
                    .or(t.scene_config.room_size)
                    .unwrap_or(self.audio_processor.config_room_size());
                self.audio_processor.reverb_params_for(&t.scene_config, rs)
            })
            .collect();
        self.audio_processor.prewarm_reverb(&reverb_params);

        // Semaphore 設定
        let sems: HashMap<String, Arc<Semaphore>> = {
            let mut m = HashMap::new();
            m.insert("voicevox".to_string(), Arc::new(Semaphore::new(self.concurrency.voicevox)));
            m.insert("aivis".to_string(), Arc::new(Semaphore::new(self.concurrency.aivis)));
            m.insert("xtts".to_string(), Arc::new(Semaphore::new(self.concurrency.xtts)));
            m
        };
        let proc_sem = Arc::new(Semaphore::new(self.concurrency.audio_process));

        let engine_manager = Arc::clone(&self.engine_manager);
        let audio_processor = Arc::clone(&self.audio_processor);

        // 各タスクを並列実行
        let mut handles = Vec::with_capacity(tasks.len());
        for (si, ii, mut task) in tasks {
            let sems = sems.clone();
            let proc_sem = Arc::clone(&proc_sem);
            let em = Arc::clone(&engine_manager);
            let ap = Arc::clone(&audio_processor);
            let ev_tx = events.clone();
            let cancel_flag = cancel.clone();
            let done = Arc::clone(&done);
            let task_text = task.text.clone();
            let task_cast_name = task.cast.name.clone();

            let handle = tokio::spawn(async move {
                if is_cancelled(&cancel_flag) {
                    return Ok(task); // 合成せず即返す
                }

                let engine_sem = sems.get(&task.cast.engine_type)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Semaphore::new(2)));

                // 合成
                let _permit = engine_sem.acquire().await.unwrap();
                if let Err(e) = em.synthesize(&task.text, &task.cast, &task.raw_path).await {
                    error!("合成失敗 {}: {e}", task.raw_path.display());
                    return Err(format!("合成失敗: {e}"));
                }
                drop(_permit);

                // 音響処理 (blocking → spawn_blocking)
                let _proc_permit = proc_sem.acquire().await.unwrap();
                let raw = task.raw_path.clone();
                let fin = task.final_path.clone();
                let cast = task.cast.clone();
                let sc = task.scene_config.clone();
                let ap2 = Arc::clone(&ap);
                let result = tokio::task::spawn_blocking(move || {
                    ap2.process(&raw, &fin, &cast, &sc)
                }).await;

                let _ = std::fs::remove_file(&task.raw_path);

                match result {
                    Ok(Ok(n)) => {
                        task.duration_ms = n as f64 / ap.config_sample_rate() as f64 * 1000.0;
                        info!("完了: {} ({:.0}ms)", task.final_path.display(), task.duration_ms);
                        let d = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        emit(&ev_tx, ProduceEvent::ItemFinished { done: d, total });
                        Ok(task)
                    }
                    Ok(Err(e)) => {
                        error!("音響処理失敗: {e}");
                        Err(format!("音響処理失敗: {e}"))
                    }
                    Err(e) => {
                        error!("spawn_blocking パニック: {e}");
                        Err(format!("音響処理パニック: {e}"))
                    }
                }
            });
            handles.push((si, ii, task_text, task_cast_name, handle));
        }

        // タスク完了を収集。失敗したタスクは task_map へ登録せず、
        // タイムライン・書き出しの対象から除外する（欠落を成功と誤認させない）。
        let mut task_map: HashMap<(usize, usize), SynthTask> = HashMap::new();
        let mut failures: Vec<TaskFailure> = Vec::new();
        for (si, ii, text, cast_name, handle) in handles {
            match handle.await {
                Ok(Ok(task)) => { task_map.insert((si, ii), task); }
                Ok(Err(reason)) => failures.push(TaskFailure { text, cast_name, reason }),
                Err(e) => {
                    error!("タスクパニック: {e}");
                    failures.push(TaskFailure { text, cast_name, reason: format!("タスクパニック: {e}") });
                }
            }
        }
        if is_cancelled(&cancel) {
            anyhow::bail!("ユーザーによりキャンセルされました");
        }
        info!("Phase2完了: 全音声の合成・処理が終わりました。（失敗 {} 件）", failures.len());

        // ── Phase 3: タイムライン構築 ──────────────────────────────────────
        emit(&events, ProduceEvent::Phase("タイムライン".into()));
        let pause_config = scenes.first().map(|s| s.pause_config.clone()).unwrap_or_default();
        let mut timeline = TimelineProcessor::new(&pause_config);
        let mut last_cast: Option<String> = None;

        for (si, scene) in scenes.iter().enumerate() {
            info!("--- Timeline Build: {} ---", scene.config.name);
            let items = &scene.items;
            let mut ii = 0;

            while ii < items.len() {
                match &items[ii] {
                    ScriptItem::Command(ScriptCommand::Parallel(n)) => {
                        let anchor = timeline.current_ms;
                        let mut occupied = Vec::new();
                        for j in 1..=*n {
                            if ii + j >= items.len() { break; }
                            let task = task_map.get(&(si, ii + j));
                            if let Some(t) = task {
                                let delay = t.cast.params.get("delay")
                                    .and_then(|v| v.as_f64()).unwrap_or(0.0);
                                timeline.register_audio(
                                    t.final_path.clone(), t.duration_ms,
                                    anchor + delay,
                                    t.text.clone(), t.display_text.clone(),
                                    t.cast.name.clone(),
                                );
                                occupied.push(delay + t.duration_ms);
                            }
                        }
                        if let Some(&max) = occupied.iter().reduce(|a, b| if a > b { a } else { b }) {
                            timeline.advance_after_parallel(anchor, max, None);
                        }
                        last_cast = None;
                        ii += 1 + n;
                        continue;
                    }
                    ScriptItem::Speech { cast_name, .. } => {
                        if let Some(t) = task_map.get(&(si, ii)) {
                            let delay = t.cast.params.get("delay")
                                .and_then(|v| v.as_f64()).unwrap_or(0.0);
                            let start = timeline.current_ms + delay;
                            timeline.register_audio(
                                t.final_path.clone(), t.duration_ms,
                                start,
                                t.text.clone(), t.display_text.clone(),
                                t.cast.name.clone(),
                            );
                            let pause = speech_pause(last_cast.as_deref(), cast_name, timeline.cast_pause_ms);
                            last_cast = Some(cast_name.clone());
                            timeline.advance_after_speech(t.duration_ms, pause);
                        } else {
                            last_cast = None;
                        }
                    }
                    ScriptItem::Command(cmd) => {
                        match cmd {
                            ScriptCommand::Pause(ms) => timeline.advance_pause(*ms),
                            ScriptCommand::Paragraph => {
                                timeline.register_paragraph();
                                timeline.advance_paragraph();
                            }
                            ScriptCommand::BgmStart(path) => timeline.register_bgm(PathBuf::from(path)),
                            ScriptCommand::BgmStop => timeline.register_bgm_stop(),
                            ScriptCommand::Se(path) => timeline.register_se(PathBuf::from(path)),
                            ScriptCommand::Parallel(_) => unreachable!(),
                        }
                    }
                }
                ii += 1;
            }
        }
        info!("Phase3完了: タイムライン構築が終わりました。");

        // エクスポート
        emit(&events, ProduceEvent::Phase("書き出し".into()));
        let timeline_events = timeline.into_events();
        let exporter = Exporter::new(&timeline_events, &self.project_root, self.sample_rate, self.bgm_config.clone())
            .with_fcpxml_fps(self.fcpxml_fps);
        exporter.generate_srt(&suffix)?;
        exporter.generate_fcpxml(&suffix)?;
        exporter.generate_combined_audio(&suffix)?;
        info!("--- Export Finished: {} ---", self.project_root.display());

        if !failures.is_empty() {
            let success = total - failures.len();
            for f in &failures {
                error!("台詞欠落: cast={} text={:?} reason={}", f.cast_name, f.text, f.reason);
            }
            anyhow::bail!(
                "{total}件中{}件の音声生成に失敗しました（成功{success}件）。詳細はログを参照してください。",
                failures.len()
            );
        }

        emit(&events, ProduceEvent::Finished);
        Ok(())
    }
}

#[cfg(test)]
mod http_client_timeout_tests {
    use super::*;
    use std::time::Duration;

    /// 接続は受け付けるが応答を一切返さないTCPリスナーを立てる。
    /// テストプロセス終了まで生存すればよいので、リスナーとスレッドはリークさせる。
    fn spawn_hanging_server() -> std::net::SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                // 応答を返さず接続を握ったままにする
                std::thread::sleep(Duration::from_secs(60));
                drop(stream);
            }
        });
        addr
    }

    #[tokio::test]
    async fn http_client_enforces_request_timeout_against_hanging_server() {
        let addr = spawn_hanging_server();
        let http_config = s2v_core::HttpConfig { connect_timeout_s: 1, request_timeout_s: 1 };
        let client = build_http_client(&http_config);

        let url = format!("http://{addr}/version");
        // クライアント自身のタイムアウトより十分長い外側ガード。
        // 外側が先に発火したら「クライアントにタイムアウトが効いていない」ことを意味する。
        let outer = tokio::time::timeout(Duration::from_secs(5), client.get(url).send()).await;

        let inner = outer.expect(
            "外側の5秒ガードより先にHTTPクライアント自身がタイムアウトすべき(request_timeout_sが効いていない)",
        );
        assert!(inner.is_err(), "応答のないサーバーに対してrequest_timeoutでErrになるべき");
    }
}

#[cfg(test)]
mod produce_events_tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn test_config() -> Config {
        // リポジトリ同梱の実 config.toml をそのまま使う（接続はしない）
        toml::from_str(include_str!("../config.toml")).unwrap()
    }

    #[tokio::test]
    async fn cancel_flag_aborts_produce_without_synthesis() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config();
        let em = std::sync::Arc::new(s2v_engines::EngineManager::new()); // エンジン未登録
        let producer = Producer::new(std::sync::Arc::clone(&em), &config, tmp.path()).unwrap();

        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser
            .parse_str("@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n")
            .unwrap();

        let cancel = std::sync::Arc::new(AtomicBool::new(true)); // 最初からキャンセル済み
        let (tx, rx) = std::sync::mpsc::channel();
        let result = producer.produce_with_events(&scenes, Some(tx), Some(cancel)).await;

        let err = result.expect_err("キャンセル時は Err");
        assert!(err.to_string().contains("キャンセル"), "実際: {err}");
        // 合成はスキップされるので ItemFinished は1件も来ない
        let events: Vec<ProduceEvent> = rx.try_iter().collect();
        assert!(!events.iter().any(|e| matches!(e, ProduceEvent::ItemFinished { .. })));
    }

    #[tokio::test]
    async fn synth_failure_is_reported_as_error_not_silent_success() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config();
        // エンジン未登録なので、この台本の唯一の台詞は合成時に必ず失敗する。
        let em = std::sync::Arc::new(s2v_engines::EngineManager::new());
        let producer = Producer::new(std::sync::Arc::clone(&em), &config, tmp.path()).unwrap();

        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser
            .parse_str("@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n")
            .unwrap();

        let result = producer.produce(&scenes).await;

        let err = result.expect_err("合成に失敗した台詞がある場合、produce は成功扱いにしてはならない");
        assert!(err.to_string().contains('1'), "失敗件数が含まれるべき: {err}");

        // 失敗した台詞のWAVは存在しない（duration=0の欠落イベントが残ってはいけない）
        let audio_dir = tmp.path().join("audio");
        let has_wav = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .any(|e| e.path().extension().is_some_and(|x| x == "wav"));
        assert!(!has_wav, "合成失敗時はWAVファイルが残っていないはず");
    }

    /// 常に成功し、無音の短いWAVを書き出すだけのテスト用エンジン。
    struct AlwaysSucceedsEngine;

    #[async_trait::async_trait]
    impl s2v_engines::Engine for AlwaysSucceedsEngine {
        async fn activate(&self) -> anyhow::Result<()> { Ok(()) }
        async fn synthesize(&self, _text: &str, _cast: &Cast, output: &std::path::Path) -> anyhow::Result<()> {
            let spec = hound::WavSpec {
                channels: 1,
                sample_rate: 24000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            };
            let mut writer = hound::WavWriter::create(output, spec)?;
            for _ in 0..2400 {
                writer.write_sample(0i16)?;
            }
            writer.finalize()?;
            Ok(())
        }
    }

    #[tokio::test]
    async fn partial_failure_excludes_failed_line_from_export_but_keeps_successful_one() {
        let _ = tracing_subscriber::fmt().with_test_writer().try_init();
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config();
        let mut em = s2v_engines::EngineManager::new();
        em.register("aivis", std::sync::Arc::new(AlwaysSucceedsEngine));
        let em = std::sync::Arc::new(em);
        let producer = Producer::new(std::sync::Arc::clone(&em), &config, tmp.path()).unwrap();

        let mut parser = s2v_core::ScriptParser::new();
        // A は voicevox(未登録) で必ず失敗、B は aivis(登録済み) で必ず成功する。
        let scenes = parser
            .parse_str(
                "@scene テスト room_size=0.1\n@cast\nA:話者A:ノーマル,voicevox,pan=0\n\nB:話者B:ノーマル,aivis,pan=0\n\n@script\nA:失敗する台詞\nB:成功する台詞\n",
            )
            .unwrap();

        let result = producer.produce(&scenes).await;
        let err = result.expect_err("一部失敗があるので produce は Err を返すべき");
        assert!(err.to_string().contains('1'), "失敗1件が含まれるべき: {err}");

        // 成功した1件分のWAVのみ残る
        let audio_dir = tmp.path().join("audio");
        let wav_count = std::fs::read_dir(&audio_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|x| x == "wav"))
            .count();
        assert_eq!(wav_count, 1, "成功した1件分のWAVのみ残るはず: 実際={wav_count}");

        // SRTには成功した台詞のみ含まれ、失敗した台詞は現れない
        let srt_path = tmp.path().join("timeline").join("subtitles.srt");
        let srt = std::fs::read_to_string(&srt_path).unwrap();
        assert!(srt.contains("成功する台詞"), "成功した台詞はSRTに含まれるべき: {srt}");
        assert!(!srt.contains("失敗する台詞"), "失敗した台詞はSRTに含まれてはいけない: {srt}");
    }
}

#[cfg(test)]
mod speech_pause_tests {
    use super::*;

    #[test]
    fn first_speech_in_scene_uses_sentence_pause() {
        // Python版: last_cast_name is None -> sentence_pause (cast_pause ではない)
        assert_eq!(speech_pause(None, "めたん", 500.0), None);
    }

    #[test]
    fn same_cast_as_previous_uses_sentence_pause() {
        assert_eq!(speech_pause(Some("めたん"), "めたん", 500.0), None);
    }

    #[test]
    fn different_cast_than_previous_uses_cast_pause() {
        assert_eq!(speech_pause(Some("めたん"), "まい", 500.0), Some(500.0));
    }
}
