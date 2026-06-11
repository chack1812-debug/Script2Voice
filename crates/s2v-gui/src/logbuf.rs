use std::collections::VecDeque;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// UI フッターに表示するログのリングバッファ。
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<String>>>,
    cap: usize,
}

impl LogBuffer {
    pub fn new(cap: usize) -> Self {
        Self { inner: Arc::new(Mutex::new(VecDeque::new())), cap }
    }

    pub fn push(&self, line: String) {
        let mut q = self.inner.lock().unwrap();
        q.push_back(line);
        while q.len() > self.cap {
            q.pop_front();
        }
    }

    pub fn lines(&self) -> Vec<String> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }
}

struct BufWriter {
    buf: LogBuffer,
    pending: Vec<u8>,
}

impl Write for BufWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.pending.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for BufWriter {
    fn drop(&mut self) {
        if let Ok(s) = String::from_utf8(std::mem::take(&mut self.pending)) {
            for l in s.lines().filter(|l| !l.trim().is_empty()) {
                self.buf.push(l.to_string());
            }
        }
    }
}

#[derive(Clone)]
struct BufMakeWriter(LogBuffer);

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for BufMakeWriter {
    type Writer = BufWriter;
    fn make_writer(&'a self) -> Self::Writer {
        BufWriter { buf: self.0.clone(), pending: Vec::new() }
    }
}

/// tracing をこのバッファへ向ける（GUI起動時に1回だけ呼ぶ）。
/// 環境変数 S2V_GUI_DEBUG が設定されている場合は stderr にも出力する（診断用）。
pub fn init_tracing(buf: LogBuffer) {
    use tracing_subscriber::prelude::*;
    let layer = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_target(false)
        .with_writer(BufMakeWriter(buf))
        .with_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        );
    let stderr_layer = std::env::var_os("S2V_GUI_DEBUG").map(|_| {
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(std::io::stderr)
            .with_filter(tracing_subscriber::EnvFilter::new("info"))
    });
    tracing_subscriber::registry().with(layer).with(stderr_layer).init();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_buffer_caps_fifo() {
        let b = LogBuffer::new(3);
        for i in 0..5 {
            b.push(format!("l{i}"));
        }
        assert_eq!(b.lines(), vec!["l2", "l3", "l4"]);
    }
}
