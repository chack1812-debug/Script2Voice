use std::collections::HashMap;
use std::path::PathBuf;
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

/// config から3エンジン（voicevox/aivis/xtts）を登録した EngineManager を構築する。
pub fn build_engine_manager(config: &Config) -> EngineManager {
    let client = Arc::new(Client::new());
    let mut em = EngineManager::new();
    em.register(
        "voicevox",
        Arc::new(HttpEngine::with_exe_path(
            "voicevox", &config.voicevox.url, Arc::clone(&client), config.voicevox.exe_path.clone(),
        )),
    );
    em.register(
        "aivis",
        Arc::new(HttpEngine::with_exe_path(
            "aivis", &config.aivis.url, Arc::clone(&client), config.aivis.exe_path.clone(),
        )),
    );
    em.register(
        "xtts",
        Arc::new(XttsEngine::with_exe_path(
            "xtts", &config.xtts.url, Arc::clone(&client), config.xtts.exe_path.clone(),
        )),
    );
    em
}

pub struct Producer {
    engine_manager: Arc<EngineManager>,
    audio_processor: Arc<AudioProcessor>,
    audio_dir: PathBuf,
    project_root: PathBuf,
    concurrency: ConcurrencyConfig,
    bgm_config: BgmConfig,
    sample_rate: u32,
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
        })
    }

    pub async fn produce(&self, scenes: &[Scene]) -> anyhow::Result<()> {
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

        // ── 出力ロック対策: 生成一式の共通連番サフィックスを決定 ───────────
        let default_files: Vec<PathBuf> = tasks.iter()
            .map(|(_, _, t)| t.final_path.clone())
            .chain([
                self.project_root.join("timeline").join("subtitles.srt"),
                self.project_root.join("timeline").join("timeline.fcpxml"),
                self.project_root.join("full_dialogue.wav"),
            ])
            .collect();
        let suffix = s2v_export::resolve_generation_suffix(&default_files, 100)?;
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

            let handle = tokio::spawn(async move {
                let engine_sem = sems.get(&task.cast.engine_type)
                    .cloned()
                    .unwrap_or_else(|| Arc::new(Semaphore::new(2)));

                // 合成
                let _permit = engine_sem.acquire().await.unwrap();
                match em.synthesize(&task.text, &task.cast, &task.raw_path).await {
                    Ok(()) => {}
                    Err(e) => {
                        error!("合成失敗 {}: {e}", task.raw_path.display());
                        return (si, ii, task);
                    }
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
                    }
                    Ok(Err(e)) => error!("音響処理失敗: {e}"),
                    Err(e) => error!("spawn_blocking パニック: {e}"),
                }
                (si, ii, task)
            });
            handles.push(handle);
        }

        // タスク完了を収集
        let mut task_map: HashMap<(usize, usize), SynthTask> = HashMap::new();
        for handle in handles {
            match handle.await {
                Ok((si, ii, task)) => { task_map.insert((si, ii), task); }
                Err(e) => error!("タスクパニック: {e}"),
            }
        }
        info!("Phase2完了: 全音声の合成・処理が終わりました。");

        // ── Phase 3: タイムライン構築 ──────────────────────────────────────
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
        let events = timeline.into_events();
        let exporter = Exporter::new(&events, &self.project_root, self.sample_rate, self.bgm_config.clone());
        exporter.generate_srt(&suffix)?;
        exporter.generate_fcpxml(&suffix)?;
        exporter.generate_combined_audio(&suffix)?;
        info!("--- Export Finished: {} ---", self.project_root.display());
        Ok(())
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
