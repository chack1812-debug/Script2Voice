use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::time::Duration;

use rodio::Source;

/// 再生開始時に音声の先頭へ挿入する無音の長さ(ms)。
/// Bluetooth ヘッドホン等はアイドルでリンクを省電力offし、再生開始の冒頭が欠落する。
/// 短い試聴クリップが丸ごと飲まれるのを防ぐため、この捨て区間でウェイクアップを吸収する。
const PREROLL_MS: u64 = 800;

/// 指定チャンネル数・サンプルレートで `ms` ミリ秒ぶんの無音ソースを作る(プリロール用)。
fn preroll_silence(channels: u16, sample_rate: u32, ms: u64) -> impl Source<Item = i16> + Send {
    rodio::source::Zero::<i16>::new(channels, sample_rate).take_duration(Duration::from_millis(ms))
}

/// 現在の OS 既定の出力デバイス名を取得する(診断ログ用)。
fn current_default_device_name() -> String {
    use rodio::cpal::traits::{DeviceTrait, HostTrait};
    rodio::cpal::default_host()
        .default_output_device()
        .and_then(|d| d.name().ok())
        .unwrap_or_else(|| "<不明>".to_string())
}

/// rodio による単一ストリーム再生(同時再生は1つ。新しい再生で前を停止)。
pub struct Player {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    sink: Option<rodio::Sink>,
    /// ストリーム生成時(起動時)に束縛された既定デバイス名。play 時の不一致検知に使う。
    bound_device: String,
}

impl Player {
    /// 出力デバイスが無い環境では None(UI 側で再生ボタンを無効化)。
    pub fn new() -> Option<Self> {
        let (stream, handle) = rodio::OutputStream::try_default().ok()?;
        let bound_device = current_default_device_name();
        tracing::info!("音声出力デバイス(起動時にバインド): {bound_device}");
        Some(Self { _stream: stream, handle, sink: None, bound_device })
    }

    pub fn play(&mut self, path: &Path) -> anyhow::Result<()> {
        self.stop();
        let file = BufReader::new(File::open(path)?);
        let source = rodio::Decoder::new(file)?;
        let (ch, sr) = (source.channels(), source.sample_rate());

        // (A) 出力先デバイスの診断: 起動時に束縛したデバイスと、現在の OS 既定が食い違うと
        // GUI は古いデバイスに出し続けるため無音になる。発生時に原因を即特定できるようログする。
        let now = current_default_device_name();
        if now != self.bound_device {
            tracing::warn!(
                "出力先デバイスが起動時と異なります(起動時='{}' 現在の既定='{}')。\
                 GUI は起動時に決めたデバイスへ再生します。音が出ない場合は GUI を再起動してください。",
                self.bound_device, now
            );
        }
        tracing::info!("再生デバイス='{}' preroll={}ms ch={ch} sr={sr}", self.bound_device, PREROLL_MS);

        let sink = rodio::Sink::try_new(&self.handle)?;
        // (B) 冒頭の欠落を吸収する無音プリロールを先に流し、続けて本体を再生する。
        sink.append(preroll_silence(ch, sr, PREROLL_MS));
        sink.append(source);
        self.sink = Some(sink);
        Ok(())
    }

    pub fn stop(&mut self) {
        if let Some(s) = self.sink.take() {
            s.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preroll_silence_has_expected_zero_samples() {
        // 2ch・48kHz・1000ms → 96000 サンプル（フレーム丸めで誤差を許容）。全て無音であること。
        let collected: Vec<i16> = preroll_silence(2, 48000, 1000).collect();
        assert!((collected.len() as i64 - 96000).abs() <= 8, "実際: {}", collected.len());
        assert!(collected.iter().all(|&x| x == 0), "プリロールは全て無音であること");
    }
}
