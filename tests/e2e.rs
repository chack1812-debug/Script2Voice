use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use s2v_core::{Cast, Config, ScriptParser};
use s2v_engines::{Engine, EngineManager};
use script2voice::Producer;

/// テスト用スタブエンジン: 実際の TTS には接続せず、短いサイン波 WAV を書き出す。
struct StubEngine;

#[async_trait]
impl Engine for StubEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn synthesize(&self, _text: &str, _cast: &Cast, output: &Path) -> anyhow::Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 24000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(output, spec)?;
        let n = (24000.0 * 0.3) as usize;
        for i in 0..n {
            let v = (2.0 * std::f32::consts::PI * 440.0 * i as f32 / 24000.0).sin();
            writer.write_sample((v * 32767.0) as i16)?;
        }
        writer.finalize()?;
        Ok(())
    }
}

const SAMPLE_SCRIPT: &str = r#"
@pause
sentence 200
cast 300
paragraph 800

@cast
めたん:四国めたん:ノーマル,voicevox,pan=-30,distance=1.0,volume=1.0

まい:まい:ノーマル,aivis,pan=30,distance=1.0,volume=1.0

@scene 01_テスト
@script
めたん:こんにちは、まいさん。
まい:こんにちは、めたんさん。
#paragraph
#pause 300
めたん:今日はいい天気ですね。
"#;

const SAMPLE_CONFIG: &str = r#"
[voicevox]
url = "http://127.0.0.1:50021"
[aivis]
url = "http://127.0.0.1:10101"
[xtts]
url = "http://localhost:8020"

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
"#;

#[tokio::test]
async fn produces_full_output_set_from_sample_script() {
    let _ = tracing_subscriber::fmt().with_test_writer().with_max_level(tracing::Level::INFO).try_init();

    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");

    let config = Config::from_toml(SAMPLE_CONFIG).unwrap();

    let mut parser = ScriptParser::new();
    let scenes = parser.parse_str(SAMPLE_SCRIPT).unwrap();
    assert!(!scenes.is_empty(), "サンプル台本が解析できること");

    let mut engine_manager = EngineManager::new();
    engine_manager.register("voicevox", Arc::new(StubEngine));
    engine_manager.register("aivis", Arc::new(StubEngine));
    engine_manager.register("xtts", Arc::new(StubEngine));
    let engine_manager = Arc::new(engine_manager);

    let producer = Producer::new(Arc::clone(&engine_manager), &config, &project_dir).unwrap();
    producer.produce(&scenes).await.unwrap();

    // SRT
    let srt_path = project_dir.join("timeline/subtitles.srt");
    assert!(srt_path.exists(), "SRT ファイルが生成されること");
    let srt = std::fs::read_to_string(&srt_path).unwrap();
    assert!(srt.contains("こんにちは、まいさん。"));
    assert!(srt.contains("-->"));
    assert!(srt.contains("[PARAGRAPH]"), "SRTに[PARAGRAPH]マーカーが含まれること");

    // FCPXML
    let fcpxml_path = project_dir.join("timeline/timeline.fcpxml");
    assert!(fcpxml_path.exists(), "FCPXML ファイルが生成されること");
    let fcpxml = std::fs::read_to_string(&fcpxml_path).unwrap();
    assert!(fcpxml.contains("<fcpxml"));
    assert!(fcpxml.contains("dialogue"));

    // ミックス済み WAV
    let mix_path = project_dir.join("full_dialogue.wav");
    assert!(mix_path.exists(), "ミックス済み WAV が生成されること");
    let reader = hound::WavReader::open(&mix_path).unwrap();
    assert_eq!(reader.spec().channels, 2);
    assert_eq!(reader.spec().sample_rate, 24000);

    // 個別音声ファイル (DSP 処理済み, audio/ 以下)
    let voice_files: Vec<_> = std::fs::read_dir(project_dir.join("audio"))
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with("voice_"))
        .collect();
    assert_eq!(voice_files.len(), 3, "3件の speech アイテムが処理されること");
}

/// 合成に時間のかかるスタブ。中断のタイミングを作るために使う。
struct SlowStubEngine;

#[async_trait]
impl Engine for SlowStubEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn synthesize(&self, _text: &str, _cast: &Cast, _output: &Path) -> anyhow::Result<()> {
        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        Ok(())
    }
}

/// 生成中に future を drop する（= Ctrl+C で打ち切られた状況）と、
/// 出力ロック `.s2v_generation*.lock` が確実に解放されること。
///
/// 残ったままだと、次回以降の生成が延々と `_1`,`_2`... へフォールバックし、
/// 出力ファイル名がずれ続ける。
#[tokio::test]
async fn dropping_produce_future_releases_generation_lock() {
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");
    let config = Config::from_toml(SAMPLE_CONFIG).unwrap();
    let mut parser = ScriptParser::new();
    let scenes = parser.parse_str(SAMPLE_SCRIPT).unwrap();

    let mut engine_manager = EngineManager::new();
    engine_manager.register("voicevox", Arc::new(SlowStubEngine));
    engine_manager.register("aivis", Arc::new(SlowStubEngine));
    engine_manager.register("xtts", Arc::new(SlowStubEngine));
    let engine_manager = Arc::new(engine_manager);

    let producer = Producer::new(Arc::clone(&engine_manager), &config, &project_dir).unwrap();
    let lock_path = project_dir.join(".s2v_generation.lock");

    let mut fut = Box::pin(producer.produce(&scenes));
    let progressed = tokio::time::timeout(std::time::Duration::from_secs(2), &mut fut).await;
    assert!(progressed.is_err(), "合成中で未完了のはず（スタブが30秒待つ）");
    assert!(lock_path.exists(), "生成中は出力ロックを保持しているはず: {}", lock_path.display());

    drop(fut); // Ctrl+C 相当の打ち切り
    assert!(
        !lock_path.exists(),
        "打ち切り時に出力ロックが解放されるべき: {}",
        lock_path.display()
    );
}

/// 走り出した合成を記録するスタブ。`synthesize` は少し待ってから完了を数える。
struct CountingSlowEngine {
    finished: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl Engine for CountingSlowEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn synthesize(&self, _text: &str, _cast: &Cast, _output: &Path) -> anyhow::Result<()> {
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        self.finished.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

/// 生成を途中で打ち切ったら、既に走り出していた合成タスクも止まること。
///
/// 止めないと、中断したはずなのに裏でエンジンを叩き続け、出力フォルダに音声ファイルが
/// 遅れて現れる（GUI はプロセスが残り続けるので特に目に見える）。
#[tokio::test]
async fn dropping_produce_future_stops_in_flight_synthesis_tasks() {
    use std::sync::atomic::{AtomicUsize, Ordering};

    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");
    let config = Config::from_toml(SAMPLE_CONFIG).unwrap();
    let mut parser = ScriptParser::new();
    let scenes = parser.parse_str(SAMPLE_SCRIPT).unwrap();

    let finished = Arc::new(AtomicUsize::new(0));
    let mut engine_manager = EngineManager::new();
    for name in ["voicevox", "aivis", "xtts"] {
        engine_manager.register(
            name,
            Arc::new(CountingSlowEngine { finished: Arc::clone(&finished) }),
        );
    }
    let engine_manager = Arc::new(engine_manager);

    let producer = Producer::new(Arc::clone(&engine_manager), &config, &project_dir).unwrap();
    let mut fut = Box::pin(producer.produce(&scenes));

    // 合成タスクが走り出したところで打ち切る（スタブは 300ms 待つのでまだ未完了）。
    let progressed = tokio::time::timeout(std::time::Duration::from_millis(150), &mut fut).await;
    assert!(progressed.is_err(), "合成中で未完了のはず");
    assert_eq!(finished.load(Ordering::SeqCst), 0, "前提: まだ1件も合成が完了していない");

    drop(fut); // Ctrl+C / GUI キャンセル相当の打ち切り

    // スタブの待ち時間より十分長く待ち、それでも完了が増えないことを確かめる。
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(
        finished.load(Ordering::SeqCst),
        0,
        "打ち切り後に合成タスクが走り続けてはいけない"
    );
}
