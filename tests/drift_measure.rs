//! ドリフト計測テスト (subtitles.srt vs full_dialogue.wav)
//!
//! 目的: 生成ファイルそのもの (subtitles.srt と full_dialogue.wav) の間に
//! 累積する字幕/音声ズレが存在するかを実測で確認する。Filmora 等の外部要因を排除し、
//! 我々のコード内部でドリフトが発生しているか否かを yes/no で判定する。
//!
//! 計測方法:
//!  - StubEngine が呼び出しごとに長さの異なるバースト WAV を生成する。
//!    各バーストは「鋭い立ち上がり (フルスケールのトーン) + その後に低振幅の本体」とし、
//!    onset を検出しやすくする。
//!  - Producer::produce を実行し subtitles.srt / full_dialogue.wav を得る。
//!  - SRT の各字幕開始時刻 (秒) を取得。full_dialogue.wav を読み、各行について
//!    SRT 開始時刻付近で短窓 RMS の急上昇 (= 新バーストの立ち上がり) を探し、実際の onset を求める。
//!  - 「実 onset - SRT開始」をドリフトとして行番号に対する傾向を出力する。
//!
//! 注意: DSP は各クリップに ITD 由来のリードディレイ (rel_l/rel_r) を付加するため、
//! onset は SRT 開始よりわずかに遅れる定数オフセットを持つ。これは累積しない。
//! 累積ドリフトの有無こそが本テストの関心事なので、生の drift と、
//! 先頭クリップのオフセットを差し引いた「相対ドリフト」の両方を報告する。

use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use s2v_core::{Cast, Config, ScriptParser};
use s2v_engines::{Engine, EngineManager};
use script2voice::Producer;

const STUB_SR: u32 = 24000;

/// 呼び出しごとに可変長のバーストを書き出すスタブエンジン。
/// - 長さは呼び出し回数とテキスト長で変化させる (全クリップが同一長にならないように)。
/// - 各クリップは「先頭 5ms のフルスケール 880Hz トーン (鋭い onset)」+「残りは
///   低振幅 220Hz の本体」とし、ミックス内で onset を検出しやすくする。
struct StubEngine {
    calls: AtomicUsize,
}

impl StubEngine {
    fn new() -> Self {
        Self { calls: AtomicUsize::new(0) }
    }
}

#[async_trait]
impl Engine for StubEngine {
    async fn activate(&self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn synthesize(&self, text: &str, _cast: &Cast, output: &Path) -> anyhow::Result<()> {
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let n = self.calls.fetch_add(1, Ordering::SeqCst);

        // 可変長: 0.30s 〜 0.80s の範囲で行ごとに変化。
        let base = 0.30_f32;
        let vary = ((n % 7) as f32) * 0.05 + ((text.chars().count() % 5) as f32) * 0.04;
        let dur_s = base + vary; // 0.30 .. 0.78
        let total = (STUB_SR as f32 * dur_s) as usize;
        let onset = (STUB_SR as f32 * 0.005) as usize; // 5ms の鋭い onset

        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: STUB_SR,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(output, spec)?;
        for i in 0..total {
            let v = if i < onset {
                // 鋭い立ち上がり: フルスケールの高めトーン
                (2.0 * std::f32::consts::PI * 880.0 * i as f32 / STUB_SR as f32).sin()
            } else {
                // 本体: 低振幅
                0.25 * (2.0 * std::f32::consts::PI * 220.0 * i as f32 / STUB_SR as f32).sin()
            };
            writer.write_sample((v * 32767.0) as i16)?;
        }
        writer.finalize()?;
        Ok(())
    }
}

/// reverb_wet>0, 非自明な room_size を持つ config。sample_rate=48000。
const DRIFT_CONFIG: &str = r#"
[voicevox]
url = "http://127.0.0.1:50021"
[aivis]
url = "http://127.0.0.1:10101"
[xtts]
url = "http://localhost:8020"

[audio]
sample_rate = 48000
microphone_spacing = 0.2
sound_speed = 340.0
air_absorption_coeff = 0.05
room_size = 0.6
reverb_wet = 0.4
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
voicevox = 4
aivis = 4
xtts = 4
audio_process = 4

[bgm]
crossfade_s = 1.0
se_fade_out_s = 0.05
"#;

/// 長い台本を生成する (話者交代と #pause を混ぜる)。
fn build_long_script(lines: usize) -> String {
    let mut s = String::new();
    s.push_str("@pause\nsentence 200\ncast 300\nparagraph 800\n\n");
    s.push_str("@cast\n");
    s.push_str("めたん:四国めたん:ノーマル,voicevox,pan=-30,distance=1.0,volume=1.0\n");
    s.push_str("\n");
    s.push_str("まい:まい:ノーマル,aivis,pan=30,distance=1.2,volume=1.0\n\n");
    s.push_str("@scene 01_ドリフト room_size=0.6 reverb_wet=0.4\n");
    s.push_str("@script\n");
    for i in 0..lines {
        // 話者交代を混ぜる (3行ごとに交代)
        let cast = if (i / 3) % 2 == 0 { "めたん" } else { "まい" };
        // テキスト長を変えて可変長を促す
        let reps = 1 + (i % 4);
        let body = "あいうえお".repeat(reps);
        s.push_str(&format!("{cast}:第{i}行 {body}。\n"));
        // たまに #pause を挿入
        if i % 37 == 17 {
            s.push_str("#pause 700\n");
        }
        // たまに段落
        if i % 53 == 29 {
            s.push_str("#paragraph\n");
        }
    }
    s
}

/// SRT をパースし、(index, start_s, end_s, text) を返す。[PARAGRAPH] 行も含む。
fn parse_srt(content: &str) -> Vec<(usize, f64, f64, String)> {
    let mut out = Vec::new();
    let blocks = content.split("\n\n");
    for block in blocks {
        let lines: Vec<&str> = block.lines().filter(|l| !l.trim().is_empty()).collect();
        if lines.len() < 2 {
            continue;
        }
        let idx: usize = match lines[0].trim().parse() {
            Ok(v) => v,
            Err(_) => continue,
        };
        let times = lines[1];
        let parts: Vec<&str> = times.split("-->").collect();
        if parts.len() != 2 {
            continue;
        }
        let start_s = parse_srt_time(parts[0].trim());
        let end_s = parse_srt_time(parts[1].trim());
        let text = lines[2..].join(" ");
        out.push((idx, start_s, end_s, text));
    }
    out.sort_by_key(|t| t.0);
    out
}

fn parse_srt_time(s: &str) -> f64 {
    // HH:MM:SS,mmm
    let (hms, ms) = s.split_once(',').unwrap_or((s, "0"));
    let parts: Vec<&str> = hms.split(':').collect();
    let h: f64 = parts[0].parse().unwrap_or(0.0);
    let m: f64 = parts[1].parse().unwrap_or(0.0);
    let sec: f64 = parts[2].parse().unwrap_or(0.0);
    let ms: f64 = ms.parse().unwrap_or(0.0);
    h * 3600.0 + m * 60.0 + sec + ms / 1000.0
}

/// ステレオ i16 WAV を [L,R] f32 frame として読む。
fn read_wav_frames(path: &Path) -> (u32, Vec<[f32; 2]>) {
    let mut reader = hound::WavReader::open(path).unwrap();
    let spec = reader.spec();
    let raw: Vec<f32> = reader.samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
    let frames: Vec<[f32; 2]> = if spec.channels == 2 {
        raw.chunks(2).map(|c| [c[0], c.get(1).copied().unwrap_or(0.0)]).collect()
    } else {
        raw.iter().map(|&s| [s, s]).collect()
    };
    (spec.sample_rate, frames)
}

/// frame 列の (L^2+R^2) パワーを返す。
fn mono_power(frames: &[[f32; 2]]) -> Vec<f32> {
    frames.iter().map(|f| f[0] * f[0] + f[1] * f[1]).collect()
}

/// 短窓 RMS の急上昇で onset を検出する。
///
/// expected_sample 付近 (±search_half) を走査し、
/// 「前方 win サンプルの平均パワー」に対して「後方 win サンプルの平均パワー」が
/// jump_ratio 倍以上、かつ絶対しきい値を超える最初の位置を onset とする。
/// バースト先頭は full-scale トーンなので、低振幅本体やリバーブ尾よりはるかに大きく、
/// 急峻なジャンプとして検出できる。見つからなければ None。
fn detect_onset(
    power: &[f32],
    expected_sample: usize,
    search_half: usize,
    win: usize,
) -> Option<usize> {
    let start = expected_sample.saturating_sub(search_half);
    let end = (expected_sample + search_half).min(power.len().saturating_sub(win + 1));
    if start + win >= power.len() {
        return None;
    }
    // 探索区間内のピークパワーを基準に、絶対しきい値を決める
    let region_end = (expected_sample + search_half).min(power.len());
    let region_start = start;
    let peak = power[region_start..region_end].iter().cloned().fold(0.0_f32, f32::max);
    if peak <= 0.0 {
        return None;
    }
    let abs_thresh = peak * 0.10; // バースト onset はピークの少なくとも10%超

    let mut best: Option<usize> = None;
    let mut i = start.max(win);
    while i < end {
        let before: f32 = power[i - win..i].iter().sum::<f32>() / win as f32;
        let after: f32 = power[i..i + win].iter().sum::<f32>() / win as f32;
        if after > abs_thresh && after > before * 4.0 + 1e-9 {
            best = Some(i);
            break;
        }
        i += 1;
    }
    best
}

#[tokio::test]
async fn measure_internal_drift_srt_vs_full_dialogue() {
    let _ = tracing_subscriber::fmt()
        .with_test_writer()
        .with_max_level(tracing::Level::WARN)
        .try_init();

    // 行数は環境変数で上書き可 (既定200)。長尺検証は DRIFT_LINES=600 等を指定。
    let n_lines: usize = std::env::var("DRIFT_LINES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(200);
    let dir = tempfile::tempdir().unwrap();
    let project_dir = dir.path().join("project");

    let config = Config::from_toml(DRIFT_CONFIG).unwrap();

    let script = build_long_script(n_lines);
    let mut parser = ScriptParser::new();
    let scenes = parser.parse_str(&script).unwrap();
    assert!(!scenes.is_empty());

    let mut em = EngineManager::new();
    em.register("voicevox", Arc::new(StubEngine::new()));
    em.register("aivis", Arc::new(StubEngine::new()));
    em.register("xtts", Arc::new(StubEngine::new()));
    let em = Arc::new(em);

    let producer = Producer::new(Arc::clone(&em), &config, &project_dir).unwrap();
    producer.produce(&scenes).await.unwrap();

    let srt_path = project_dir.join("timeline/subtitles.srt");
    let mix_path = project_dir.join("full_dialogue.wav");
    assert!(srt_path.exists(), "SRT が生成されること");
    assert!(mix_path.exists(), "full_dialogue.wav が生成されること");

    let srt = std::fs::read_to_string(&srt_path).unwrap();
    let entries = parse_srt(&srt);
    // [PARAGRAPH] 以外の発話字幕のみを対象にする
    let speech_entries: Vec<&(usize, f64, f64, String)> =
        entries.iter().filter(|e| !e.3.contains("[PARAGRAPH]")).collect();

    let (sr, frames) = read_wav_frames(&mix_path);
    assert_eq!(sr, 48000, "ミックスは config sample_rate(48000) で出力されるはず");
    let power = mono_power(&frames);

    let total_wav_s = frames.len() as f64 / sr as f64;
    let last_srt_end_s = entries.iter().map(|e| e.2).fold(0.0_f64, f64::max);

    // onset 探索パラメータ
    let search_half = (sr as f64 * 0.40) as usize; // ±400ms
    let win = (sr as f64 * 0.001) as usize; // 1ms 窓

    println!("\n==== DRIFT MEASUREMENT (subtitles.srt vs full_dialogue.wav) ====");
    println!("lines requested            : {n_lines}");
    println!("speech subtitle entries    : {}", speech_entries.len());
    println!("mix sample_rate            : {sr}");
    println!("mix total duration (s)     : {total_wav_s:.3}");
    println!("SRT last end timestamp (s) : {last_srt_end_s:.3}");
    println!(
        "mix_total - srt_last_end   : {:.3} s  (mix should be >= srt end by ~reverb tail)",
        total_wav_s - last_srt_end_s
    );

    // 各発話行について onset を検出し drift を計算
    let mut measured: Vec<(usize, f64, f64, f64)> = Vec::new(); // (srt_index, srt_start, onset_s, drift_ms)
    let mut undetected = 0usize;
    for e in &speech_entries {
        let (idx, srt_start_s, _end, _text) = e;
        let expected = (*srt_start_s * sr as f64) as usize;
        match detect_onset(&power, expected, search_half, win) {
            Some(onset_sample) => {
                let onset_s = onset_sample as f64 / sr as f64;
                let drift_ms = (onset_s - *srt_start_s) * 1000.0;
                measured.push((*idx, *srt_start_s, onset_s, drift_ms));
            }
            None => undetected += 1,
        }
    }

    println!("detected onsets            : {} / {}", measured.len(), speech_entries.len());
    println!("undetected (skipped)       : {undetected}");
    assert!(
        measured.len() as f64 >= speech_entries.len() as f64 * 0.8,
        "大半の行で onset を検出できること (検出 {} / {})",
        measured.len(),
        speech_entries.len()
    );

    // 先頭クリップの drift を「定数 DSP オフセット」とみなし、相対ドリフトを算出
    let baseline_ms = measured.first().map(|m| m.3).unwrap_or(0.0);

    // 統計
    let max_abs_drift = measured.iter().map(|m| m.3.abs()).fold(0.0_f64, f64::max);
    let max_abs_rel = measured.iter().map(|m| (m.3 - baseline_ms).abs()).fold(0.0_f64, f64::max);

    // 50ms を超える最初の行 (相対ドリフト基準)
    let first_rel_over_50 = measured.iter().find(|m| (m.3 - baseline_ms).abs() > 50.0);
    let first_raw_over_50 = measured.iter().find(|m| m.3.abs() > 50.0);

    // 単調増加チェック: 前半 vs 後半の平均相対ドリフト
    let half = measured.len() / 2;
    let avg_first_half: f64 =
        measured[..half].iter().map(|m| m.3 - baseline_ms).sum::<f64>() / half.max(1) as f64;
    let avg_second_half: f64 = measured[half..]
        .iter()
        .map(|m| m.3 - baseline_ms)
        .sum::<f64>()
        / (measured.len() - half).max(1) as f64;

    // 線形回帰の傾き (相対ドリフト vs 行番号インデックス)
    let nfit = measured.len() as f64;
    let mean_x = (measured.len() as f64 - 1.0) / 2.0;
    let mean_y: f64 = measured.iter().map(|m| m.3 - baseline_ms).sum::<f64>() / nfit;
    let mut num = 0.0;
    let mut den = 0.0;
    for (i, m) in measured.iter().enumerate() {
        let x = i as f64 - mean_x;
        num += x * ((m.3 - baseline_ms) - mean_y);
        den += x * x;
    }
    let slope_ms_per_line = if den > 0.0 { num / den } else { 0.0 };

    println!("\n---- per-line drift summary ----");
    println!("baseline drift (line0, ms) : {baseline_ms:.2}  (constant DSP lead-delay offset)");
    println!("max |raw drift| (ms)       : {max_abs_drift:.2}");
    println!("max |relative drift| (ms)  : {max_abs_rel:.2}  (raw minus baseline)");
    println!("avg rel drift 1st half (ms): {avg_first_half:.2}");
    println!("avg rel drift 2nd half (ms): {avg_second_half:.2}");
    println!("regression slope (ms/line) : {slope_ms_per_line:.4}");
    match first_raw_over_50 {
        Some(m) => println!("first |raw drift|>50ms     : srt#{} at {:.2}s (drift {:.1}ms)", m.0, m.1, m.3),
        None => println!("first |raw drift|>50ms     : NONE"),
    }
    match first_rel_over_50 {
        Some(m) => println!(
            "first |rel drift|>50ms     : srt#{} at {:.2}s (rel {:.1}ms)",
            m.0,
            m.1,
            m.3 - baseline_ms
        ),
        None => println!("first |rel drift|>50ms     : NONE"),
    }

    // サンプル抽出して出力 (先頭/中間/末尾の数行)
    println!("\n---- sampled rows (idx | srt_start_s | onset_s | raw_drift_ms | rel_drift_ms) ----");
    let sample_indices: Vec<usize> = {
        let m = measured.len();
        let mut v = vec![0usize, 1, 2];
        v.push(m / 4);
        v.push(m / 2);
        v.push(3 * m / 4);
        v.push(m.saturating_sub(3));
        v.push(m.saturating_sub(2));
        v.push(m.saturating_sub(1));
        v.sort_unstable();
        v.dedup();
        v.retain(|&i| i < m);
        v
    };
    for &i in &sample_indices {
        let (idx, srt_start, onset_s, drift_ms) = measured[i];
        println!(
            "  {idx:>4} | {srt_start:>10.3} | {onset_s:>8.3} | {drift_ms:>10.2} | {:>10.2}",
            drift_ms - baseline_ms
        );
    }

    // 判定: 相対ドリフトが時間とともに有意に増大していないこと。
    // しきい値: 末尾でも相対ドリフトが 50ms 未満、回帰傾きが ~0。
    let grows = avg_second_half.abs() > avg_first_half.abs() + 30.0 && slope_ms_per_line.abs() > 0.05;
    println!(
        "\nVERDICT: {}",
        if max_abs_rel < 50.0 && !grows {
            "NO internal cumulative drift detected (SRT and WAV are locked)."
        } else {
            "INTERNAL DRIFT DETECTED (see numbers above)."
        }
    );
    println!("================================================================\n");

    // テストの assert は「累積ドリフト無し」を主張する。万一ドリフトがあれば失敗して可視化される。
    assert!(
        max_abs_rel < 50.0,
        "相対ドリフト(累積)が 50ms を超えた: max_abs_rel={max_abs_rel:.2}ms slope={slope_ms_per_line:.4}ms/line"
    );
}
