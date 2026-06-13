//! 再生(rodio)経路だけを GUI 抜きで検証する診断プローブ。
//!
//! GUI の `audio_play.rs` / `transport.rs` と同じ rodio の使い方で実音を出し、
//! 「再生開始ログは出るのに音が聞こえない」問題の切り分けに使う。
//!
//! 実行:
//!   cargo run -p s2v-gui --example audio_probe              # 440Hz テストトーンを2秒再生
//!   cargo run -p s2v-gui --example audio_probe -- path.wav  # 指定 WAV を再生(ピークも表示)
//!
//! 見るポイント:
//!   - 「既定の出力デバイス」が、実際に音を聞いているスピーカー/ヘッドホンか？
//!   - テストトーンが聞こえるか？ → 聞こえれば rodio・デバイスは正常、問題は WAV 内容か GUI 統合。
//!                                  聞こえなければデバイス/既定出力先の問題(ユーザー仮説どおり)。

use std::io::BufReader;
use std::time::Duration;

use rodio::cpal::traits::{DeviceTrait, HostTrait};
use rodio::Source;

fn main() -> anyhow::Result<()> {
    // ── 1) 出力デバイスの列挙 ─────────────────────────────
    let host = rodio::cpal::default_host();
    println!("オーディオホスト: {}", host.id().name());
    match host.default_output_device() {
        Some(d) => println!(
            "★ 既定の出力デバイス: {}",
            d.name().unwrap_or_else(|_| "<名前取得失敗>".into())
        ),
        None => println!("!! 既定の出力デバイスがありません(ここが原因の可能性)"),
    }
    println!("--- 利用可能な出力デバイス一覧 ---");
    match host.output_devices() {
        Ok(devs) => {
            for (i, d) in devs.enumerate() {
                println!("  [{i}] {}", d.name().unwrap_or_else(|_| "<名前取得失敗>".into()));
            }
        }
        Err(e) => println!("  列挙失敗: {e}"),
    }

    // ── 2) GUI と同じ rodio 出力ストリームを開く ───────────
    let (_stream, handle) = rodio::OutputStream::try_default()?;
    println!("OutputStream::try_default() 成功(GUI の Player::new と同じ経路)");
    let sink = rodio::Sink::try_new(&handle)?;

    // ── 3) トーン or 指定 WAV を再生 ──────────────────────
    let args: Vec<String> = std::env::args().collect();
    if let Some(path) = args.get(1) {
        // 指定 WAV: まずピークを確認(無音データでないか)してから再生する。
        let dec = rodio::Decoder::new(BufReader::new(std::fs::File::open(path)?))?;
        let (ch, sr) = (dec.channels(), dec.sample_rate());
        let samples: Vec<i16> = dec.collect();
        let peak = samples.iter().map(|s| s.unsigned_abs() as u32).max().unwrap_or(0);
        println!(
            "WAV: {path}\n  channels={ch}  sample_rate={sr}  samples={}  peak={peak}/32767 ({:.1} dBFS)",
            samples.len(),
            if peak == 0 { f64::NEG_INFINITY } else { 20.0 * (peak as f64 / 32767.0).log10() },
        );
        if peak == 0 {
            println!("!! ピークが 0 = この WAV は無音データです(再生経路ではなく生成側の問題)");
        }
        // 再生用にデコードし直して append(上の collect で消費済みのため)
        sink.append(rodio::Decoder::new(BufReader::new(std::fs::File::open(path)?))?);
        println!(">> この WAV を再生します。音は聞こえますか？");
    } else {
        let tone = rodio::source::SineWave::new(440.0)
            .take_duration(Duration::from_secs(2))
            .amplify(0.20);
        sink.append(tone);
        println!(">> 440Hz のテストトーンを2秒再生します。音は聞こえますか？");
    }

    sink.sleep_until_end();
    println!("再生完了。");
    Ok(())
}
