use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// rodio による単一ストリーム再生(同時再生は1つ。新しい再生で前を停止)。
pub struct Player {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    sink: Option<rodio::Sink>,
}

impl Player {
    /// 出力デバイスが無い環境では None(UI 側で再生ボタンを無効化)。
    pub fn new() -> Option<Self> {
        let (stream, handle) = rodio::OutputStream::try_default().ok()?;
        Some(Self { _stream: stream, handle, sink: None })
    }

    pub fn play(&mut self, path: &Path) -> anyhow::Result<()> {
        self.stop();
        let file = BufReader::new(File::open(path)?);
        let source = rodio::Decoder::new(file)?;
        let sink = rodio::Sink::try_new(&self.handle)?;
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
