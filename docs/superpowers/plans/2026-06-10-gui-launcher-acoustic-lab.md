# GUI（ランチャー＋音響ラボ）実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 台本選択・行プレビュー・一括実行・音響ラボ（プリセット＋任意WAV＋試聴履歴A/B）を持つ egui GUI（`crates/s2v-gui`）を追加する。

**Architecture:** 既存パイプラインの「別の入口」。GUI は s2v-core（パース）・s2v-engines（TTS）・s2v-audio（DSP）・script2voice lib（一括実行）をそのまま呼ぶ。UI スレッド＋tokio ランタイム（バックグラウンド）＋`std::sync::mpsc` で結果を UI に返す。台本編集は外部エディタ（mtime ポーリングで自動再読込）。

**Tech Stack:** eframe/egui 0.29, rfd 0.15（ネイティブダイアログ）, rodio 0.19（再生）, toml 0.8, tempfile（一時WAV置き場）。

**Spec:** `docs/superpowers/specs/2026-06-10-gui-launcher-acoustic-lab-design.md`（Beads: s2v-6p2）

**設計からの確定変更**（spec に反映済み）:
- `process_buffer` API は**追加しない**。ラボも行プレビューも既存のパスベース `AudioProcessor::process()`（WAV読込→正規化→リサンプル→DSP→書出し）で足りると判明（YAGNI）。
- ファイル監視は notify でなく **mtime ポーリング（500ms）**。エディタの「一時ファイル→リネーム」保存にも単純・確実。
- 多ch WAV は既存 process() と同じ **Lch 使用**（ミックスはしない）。

**重要な既存API（変更しないもの）:**
- `EngineManager::synthesize(&self, text: &str, cast: &Cast, out: &Path) -> anyhow::Result<()>` / `activate_required(&HashSet<String>)` / `shutdown_all()`
- `AudioProcessor::new(AudioConfig)` / `process(&self, input: &Path, output: &Path, cast: &Cast, scene: &SceneConfig) -> anyhow::Result<usize>`
- `Producer::new(Arc<EngineManager>, &Config, project_root)` / `ScriptParser::new()` / `parse_str(&mut self, &str) -> anyhow::Result<Vec<Scene>>`
- `ScriptItem::Speech { cast_name, text, display_text, offset_params, scene_config }`、`Cast { name, speaker_name, engine_type, pan, distance, volume, params, height, height_offset }`、`Cast::with_offsets(&HashMap<String,f64>)`

---

### Task 1: s2v-core パース警告の構造化（ParseWarning）

**Files:**
- Modify: `crates/s2v-core/src/parser.rs`
- Modify: `crates/s2v-core/src/lib.rs`（re-export 追加）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs` の `mod tests` 内に追加:

```rust
    #[test]
    fn unknown_cast_produces_warning_with_line_number() {
        let mut p = ScriptParser::new();
        let src = "@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n誰か:こんばんは\n";
        let scenes = p.parse_str(src).unwrap();
        // 未定義キャスト行は従来どおり無視される
        let n = scenes[0].items.iter().filter(|i| matches!(i, ScriptItem::Speech { .. })).count();
        assert_eq!(n, 1);
        // 警告が行番号付きで記録される
        let w = p.warnings();
        assert_eq!(w.len(), 1);
        assert_eq!(w[0].line_no, 6);
        assert!(w[0].message.contains("誰か"));
    }

    #[test]
    fn warnings_are_reset_per_parse() {
        let mut p = ScriptParser::new();
        let src = "@scene テスト room_size=0.1\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\n誰か:こんばんは\n";
        p.parse_str(src).unwrap();
        assert_eq!(p.warnings().len(), 1);
        p.parse_str(src).unwrap();
        assert_eq!(p.warnings().len(), 1, "2回目のparseで累積しない");
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p s2v-core unknown_cast`
Expected: コンパイルエラー（`warnings` メソッド未定義）

- [ ] **Step 3: 実装**

`parser.rs` に型を追加し、`ScriptParser` にフィールドを足す:

```rust
/// パース中に検出した非致命的な問題（行は無視されるがパース自体は続行）。
#[derive(Debug, Clone, PartialEq)]
pub struct ParseWarning {
    /// 1始まりの行番号
    pub line_no: usize,
    pub message: String,
}
```

`ScriptParser` 構造体に `warnings: Vec<ParseWarning>` フィールドを追加し、`new()` で `warnings: Vec::new()` を初期化。アクセサを追加:

```rust
    pub fn warnings(&self) -> &[ParseWarning] {
        &self.warnings
    }
```

`parse_str` の冒頭（`let mut scenes` の前）に `self.warnings.clear();` を追加。行ループを行番号付きに変更:

```rust
        for (idx, line) in text.lines().enumerate() {
            let line_no = idx + 1;
            let line = line.trim();
```

`"@script"` アームの呼び出しを `self.parse_script_line(line, line_no)` に変更。`parse_script_line` のシグネチャを `fn parse_script_line(&mut self, line: &str, line_no: usize) -> Option<ScriptItem>` に変更（`&self`→`&mut self`、`line_no` 追加）。未定義キャストの分岐を:

```rust
        if !self.casts.contains_key(role) {
            self.warnings.push(ParseWarning {
                line_no,
                message: format!("キャスト「{role}」が未定義です（この行は無視されます）"),
            });
            return None;
        }
```

`crates/s2v-core/src/lib.rs` の既存 `pub use`（`ScriptParser` を公開している行）に `ParseWarning` を追加（例: `pub use parser::{ScriptParser, ParseWarning};` — 既存の記述形式に合わせる）。

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-core`
Expected: 全テスト PASS（既存テスト含む）

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/parser.rs crates/s2v-core/src/lib.rs
git commit -m "feat(core): collect parse warnings with line numbers for undefined casts"
```

---

### Task 2: script2voice lib へ resolve_config_path / build_engine_manager を移設

GUI と CLI で同じ設定解決・エンジン登録を使うための**純粋な移動リファクタ**（挙動変更なし）。

**Files:**
- Modify: `src/lib.rs`
- Modify: `src/main.rs`

- [ ] **Step 1: lib.rs に2関数を追加**

`src/lib.rs` の `use` 群に追加: `use reqwest::Client;` `use s2v_engines::{HttpEngine, XttsEngine};`（`EngineManager` は既存 use にあり）。`Producer` 定義の前に追加:

```rust
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
```

注: `Config` の use が `s2v_core::{...}` に無ければ追加する。

- [ ] **Step 2: main.rs を移設先を使う形に変更**

`src/main.rs` から `fn resolve_config_path` 本体を削除し、`use script2voice::{Producer, resolve_config_path, build_engine_manager};` に変更。エンジン登録ブロック（`let client = Arc::new(Client::new());` から `engine_manager.register("xtts", ...)` まで）を削除し:

```rust
    let engine_manager = Arc::new(build_engine_manager(&config));
```

に置換。不要になった `use reqwest::Client;` `use s2v_engines::{EngineManager, HttpEngine, XttsEngine};` を整理（`EngineManager` は `run_pipeline` シグネチャで使用継続なので残す）。`main.rs` の `mod tests` にある `resolve_config_path` 系テストは `use script2voice::resolve_config_path;` を足してそのまま動かす（テスト自体は移動不要）。

- [ ] **Step 3: ビルド・全テスト**

Run: `cargo test --workspace`
Expected: 全 PASS（純移動なので挙動不変）

- [ ] **Step 4: コミット**

```bash
git add src/lib.rs src/main.rs
git commit -m "refactor: move resolve_config_path/build_engine_manager into library for GUI reuse"
```

---

### Task 3: produce_with_events（進捗イベント＋キャンセル）

**Files:**
- Modify: `src/lib.rs`
- Modify: ルート `Cargo.toml`（dev-dependencies に toml 追加）

- [ ] **Step 1: 失敗するテストを書く**

ルート `Cargo.toml` の `[dev-dependencies]` に `toml = "0.8"` を追加。`src/lib.rs` 末尾の `#[cfg(test)] mod tests`（無ければ新設）に追加:

```rust
#[cfg(test)]
mod produce_events_tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

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
}
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test cancel_flag_aborts`
Expected: コンパイルエラー（`ProduceEvent` / `produce_with_events` 未定義）

- [ ] **Step 3: 実装**

`src/lib.rs` の use に追加: `use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};` `use std::sync::mpsc::Sender;`

`Producer` 定義の前に:

```rust
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
```

既存 `pub async fn produce(&self, scenes: &[Scene])` の**本体を** `produce_with_events` に移し、ラッパー化:

```rust
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
        // …既存の Phase1（パス割り当て）・suffix 解決コードはそのまま…
```

Phase2 開始直前（`info!("Phase1完了...")` の後）に:

```rust
        let total = tasks.len();
        let done = Arc::new(AtomicUsize::new(0));
        emit(&events, ProduceEvent::Phase("合成".into()));
```

タスク spawn ループ内、`tokio::spawn(async move {` 用のクローンに `events` の clone（`let ev_tx = events.clone();`）、`cancel` の clone（`let cancel_flag = cancel.clone();`）、`let done = Arc::clone(&done);` を追加。spawn 本体の先頭に:

```rust
                if is_cancelled(&cancel_flag) {
                    return (si, ii, task); // 合成せず即返す
                }
```

成功分岐 `Ok(Ok(n)) => { ... }` の `info!("完了: ...")` の後に:

```rust
                        let d = done.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                        emit(&ev_tx, ProduceEvent::ItemFinished { done: d, total });
```

全ハンドル join 後（Phase2完了 info! の直前）に:

```rust
        if is_cancelled(&cancel) {
            anyhow::bail!("ユーザーによりキャンセルされました");
        }
```

Phase3 開始時に `emit(&events, ProduceEvent::Phase("タイムライン".into()));`、Export 開始時に `emit(&events, ProduceEvent::Phase("書き出し".into()));`、関数末尾 `Ok(())` の直前に `emit(&events, ProduceEvent::Finished);` を追加。

注意: クロージャ内へ moveするクローン名は既存変数と衝突しないこと（`ev_tx`/`cancel_flag`）。`events`/`cancel` 本体はループ後の判定・emit に使うので move しない。

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test --workspace`
Expected: 新テスト含め全 PASS（既存 produce() 経由のE2Eも不変）

- [ ] **Step 5: コミット**

```bash
git add src/lib.rs Cargo.toml
git commit -m "feat(lib): add produce_with_events with progress events and cancel flag"
```

---

### Task 4: s2v-gui クレートの土台（ウィンドウ・日本語フォント・タブ枠・ログフッター）

**Files:**
- Modify: ルート `Cargo.toml`（workspace members に追加）
- Create: `crates/s2v-gui/Cargo.toml`
- Create: `crates/s2v-gui/src/main.rs`
- Create: `crates/s2v-gui/src/fonts.rs`
- Create: `crates/s2v-gui/src/logbuf.rs`
- Create: `crates/s2v-gui/src/app.rs`

- [ ] **Step 1: workspace へ追加**

ルート `Cargo.toml` の `members` を:

```toml
members = [".", "crates/s2v-core", "crates/s2v-engines", "crates/s2v-audio", "crates/s2v-export", "crates/s2v-gui"]
```

- [ ] **Step 2: crates/s2v-gui/Cargo.toml**

```toml
[package]
name = "s2v-gui"
version = "0.1.0"
edition = "2021"

[dependencies]
script2voice = { path = "../.." }
s2v-core = { path = "../s2v-core" }
s2v-engines = { path = "../s2v-engines" }
s2v-audio = { path = "../s2v-audio" }
anyhow.workspace = true
tokio.workspace = true
tracing.workspace = true
serde.workspace = true
eframe = "0.29"
rfd = "0.15"
rodio = "0.19"
toml = "0.8"
tempfile = "3"
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
```

- [ ] **Step 3: logbuf.rs（リングバッファ＋tracing 接続）**

```rust
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
    tracing_subscriber::registry().with(layer).init();
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
```

- [ ] **Step 4: fonts.rs（日本語フォント。egui 既定フォントは CJK 字形を持たないため必須）**

```rust
/// Windows 標準の日本語フォントを egui に登録する（無ければ豆腐になるだけで続行）。
pub fn install_japanese_fonts(ctx: &egui::Context) {
    let candidates = [
        r"C:\Windows\Fonts\meiryo.ttc",
        r"C:\Windows\Fonts\YuGothM.ttc",
        r"C:\Windows\Fonts\msgothic.ttc",
    ];
    let Some(bytes) = candidates.iter().find_map(|p| std::fs::read(p).ok()) else {
        tracing::warn!("日本語フォントが見つかりません（表示が崩れる可能性）");
        return;
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("jp".into(), egui::FontData::from_owned(bytes));
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts.families.entry(family).or_default().push("jp".into());
    }
    ctx.set_fonts(fonts);
}
```

- [ ] **Step 5: app.rs（タブ枠＋フッター。中身は後続タスクで差し込む）**

```rust
use crate::logbuf::LogBuffer;

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Script,
    Lab,
}

pub struct App {
    pub tab: Tab,
    pub log: LogBuffer,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        Self { tab: Tab::Script, log }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Script, "📜 台本");
                ui.selectable_value(&mut self.tab, Tab::Lab, "🎛 音響ラボ");
            });
        });
        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(120.0)
            .show(ctx, |ui| {
                ui.collapsing("実行ログ", |ui| {
                    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                        for line in self.log.lines() {
                            ui.monospace(line);
                        }
                    });
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| match self.tab {
            Tab::Script => {
                ui.label("(台本タブ: Task 10 で実装)");
            }
            Tab::Lab => {
                ui.label("(音響ラボ: Task 11 で実装)");
            }
        });
        // バックグラウンド完了の取りこぼし防止に定期再描画
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }
}
```

- [ ] **Step 6: main.rs**

```rust
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod fonts;
mod logbuf;

fn main() -> eframe::Result {
    let log = logbuf::LogBuffer::new(500);
    logbuf::init_tracing(log.clone());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size([1100.0, 760.0]),
        ..Default::default()
    };
    eframe::run_native(
        "Script2Voice",
        options,
        Box::new(move |cc| {
            fonts::install_japanese_fonts(&cc.egui_ctx);
            Ok(Box::new(app::App::new(cc, log)))
        }),
    )
}
```

- [ ] **Step 7: ビルド・テスト・手動スモーク**

Run: `cargo test -p s2v-gui` → PASS（logbuf テスト）
Run: `cargo run -p s2v-gui` → ウィンドウが開き、タブ「台本」「音響ラボ」が日本語で表示・切替できること、フッター「実行ログ」が開閉できることを目視確認して閉じる。

- [ ] **Step 8: コミット**

```bash
git add Cargo.toml crates/s2v-gui
git commit -m "feat(gui): scaffold s2v-gui crate (window, JP fonts, tabs, log footer)"
```

---

### Task 5: presets.rs（プリセット読込＋組込み既定）

**Files:**
- Create: `crates/s2v-gui/src/presets.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod presets;` 追加）

- [ ] **Step 1: 失敗するテストを書く**（presets.rs を作りテストのみ先に書く）

```rust
use std::path::Path;

/// 音響ラボのプリセット1件。すべて省略可（省略項目は現在のスライダー値を維持）。
#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
pub struct Preset {
    pub name: String,
    pub room_w: Option<f64>,
    pub room_d: Option<f64>,
    pub room_h: Option<f64>,
    pub listener_dx: Option<f64>,
    pub listener_dy: Option<f64>,
    pub listener_z: Option<f64>,
    pub reverb_wet: Option<f64>,
    pub pan: Option<f64>,
    pub distance: Option<f64>,
    pub height: Option<f64>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn loads_presets_from_toml_after_builtins() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.toml");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "[[preset]]\nname = \"テスト部屋\"\nroom_w = 7.0\nroom_d = 8.0\nroom_h = 3.5\n").unwrap();
        drop(f);
        let (presets, warn) = load_presets(&path);
        assert!(warn.is_none());
        assert!(presets.len() > builtin_presets().len());
        let p = presets.iter().find(|p| p.name == "テスト部屋").unwrap();
        assert_eq!(p.room_w, Some(7.0));
        assert_eq!(p.reverb_wet, None);
    }

    #[test]
    fn missing_file_returns_builtins_without_warning() {
        let (presets, warn) = load_presets(Path::new("Z:/no/such/presets.toml"));
        assert_eq!(presets, builtin_presets());
        assert!(warn.is_none());
    }

    #[test]
    fn broken_toml_returns_builtins_with_warning() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("presets.toml");
        std::fs::write(&path, "[[preset]\nname=壊れてる").unwrap();
        let (presets, warn) = load_presets(&path);
        assert_eq!(presets, builtin_presets());
        assert!(warn.is_some());
    }
}
```

- [ ] **Step 2: テストが失敗することを確認**

`main.rs` に `mod presets;` を追加してから:
Run: `cargo test -p s2v-gui presets`
Expected: コンパイルエラー（`load_presets`/`builtin_presets` 未定義）

- [ ] **Step 3: 実装**（テストの上に追加）

```rust
#[derive(serde::Deserialize)]
struct PresetFile {
    #[serde(default)]
    preset: Vec<Preset>,
}

/// presets.toml が無くても使える組込みプリセット。
pub fn builtin_presets() -> Vec<Preset> {
    fn p(name: &str, w: f64, d: f64, h: f64, dy: f64, z: f64, wet: f64) -> Preset {
        Preset {
            name: name.into(),
            room_w: Some(w), room_d: Some(d), room_h: Some(h),
            listener_dx: Some(0.0), listener_dy: Some(dy), listener_z: Some(z),
            reverb_wet: Some(wet),
            pan: None, distance: None, height: None,
        }
    }
    vec![
        p("ラジオスタジオ", 4.0, 5.0, 3.0, 0.0, 1.2, 1.0),
        p("会議室", 8.0, 12.0, 2.7, 0.0, 1.2, 1.0),
        p("2000席ホール", 25.0, 45.0, 18.0, -15.0, 1.1, 1.0),
        p("屋外風（残響なし）", 50.0, 50.0, 30.0, 0.0, 1.6, 0.0),
    ]
}

/// 組込み＋presets.toml の内容を返す。第2要素は警告（破損時のみ）。
pub fn load_presets(path: &Path) -> (Vec<Preset>, Option<String>) {
    let mut presets = builtin_presets();
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(_) => return (presets, None), // ファイル無しは正常系
    };
    match toml::from_str::<PresetFile>(&text) {
        Ok(file) => {
            presets.extend(file.preset);
            (presets, None)
        }
        Err(e) => (presets, Some(format!("presets.toml の読み込みに失敗（組込みのみ使用）: {e}"))),
    }
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-gui presets` → PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/presets.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): preset loading with builtin defaults and broken-file fallback"
```

---

### Task 6: scene_line.rs（ラボパラメータ⇔SceneConfig/Cast/@scene行）

**Files:**
- Create: `crates/s2v-gui/src/scene_line.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod scene_line;` 追加）

- [ ] **Step 1: 失敗するテストを書く**

```rust
use s2v_core::{Cast, SceneConfig};
use std::collections::HashMap;

/// 音響ラボのスライダー状態。
#[derive(Debug, Clone, PartialEq)]
pub struct LabParams {
    pub room_w: f64,
    pub room_d: f64,
    pub room_h: f64,
    pub listener_dx: f64,
    pub listener_dy: f64,
    pub listener_z: f64,
    pub reverb_wet: f64,
    pub pan: f64,
    pub distance: f64,
    pub height: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_line_roundtrips_through_parser() {
        let mut p = LabParams::default();
        p.room_w = 25.0;
        p.room_d = 45.0;
        p.room_h = 18.0;
        p.listener_dy = -15.0;
        p.listener_z = 1.1;
        p.reverb_wet = 0.5;
        let line = p.scene_line("ラボ");
        // 生成した @scene 行を実パーサに通して値が一致することを確認
        let src = format!("{line}\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:あ\n");
        let scenes = s2v_core::ScriptParser::new().parse_str(&src).unwrap();
        let sc = &scenes[0].config;
        assert_eq!(sc.name, "ラボ");
        assert_eq!(sc.room_w, Some(25.0));
        assert_eq!(sc.room_d, Some(45.0));
        assert_eq!(sc.room_h, Some(18.0));
        assert_eq!(sc.listener_dx, Some(0.0));
        assert_eq!(sc.listener_dy, Some(-15.0));
        assert_eq!(sc.listener_z, Some(1.1));
        assert_eq!(sc.reverb_wet, Some(0.5));
    }

    #[test]
    fn apply_preset_overrides_only_given_fields() {
        let mut p = LabParams::default();
        p.pan = 30.0;
        let preset = crate::presets::builtin_presets()
            .into_iter()
            .find(|p| p.name == "2000席ホール")
            .unwrap();
        p.apply_preset(&preset);
        assert_eq!(p.room_w, 25.0);
        assert_eq!(p.listener_dy, -15.0);
        assert_eq!(p.pan, 30.0, "preset に無い項目は維持");
    }

    #[test]
    fn to_cast_and_scene_config_carry_values() {
        let p = LabParams::default();
        let c = p.to_cast();
        assert_eq!(c.distance, 1.0);
        assert_eq!(c.height, Some(1.2));
        let sc = p.to_scene_config("x");
        assert_eq!(sc.room_size, None, "寸法直接指定なので room_size は使わない");
        assert_eq!(sc.room_w, Some(4.0));
    }
}
```

- [ ] **Step 2: 失敗確認**

`main.rs` に `mod scene_line;` 追加後:
Run: `cargo test -p s2v-gui scene_line`
Expected: コンパイルエラー

- [ ] **Step 3: 実装**

```rust
impl Default for LabParams {
    fn default() -> Self {
        Self {
            room_w: 4.0, room_d: 5.0, room_h: 3.0,
            listener_dx: 0.0, listener_dy: 0.0, listener_z: 1.2,
            reverb_wet: 1.0,
            pan: 0.0, distance: 1.0, height: 1.2,
        }
    }
}

impl LabParams {
    pub fn apply_preset(&mut self, p: &crate::presets::Preset) {
        if let Some(v) = p.room_w { self.room_w = v; }
        if let Some(v) = p.room_d { self.room_d = v; }
        if let Some(v) = p.room_h { self.room_h = v; }
        if let Some(v) = p.listener_dx { self.listener_dx = v; }
        if let Some(v) = p.listener_dy { self.listener_dy = v; }
        if let Some(v) = p.listener_z { self.listener_z = v; }
        if let Some(v) = p.reverb_wet { self.reverb_wet = v; }
        if let Some(v) = p.pan { self.pan = v; }
        if let Some(v) = p.distance { self.distance = v; }
        if let Some(v) = p.height { self.height = v; }
    }

    pub fn to_scene_config(&self, name: &str) -> SceneConfig {
        let mut sc = SceneConfig::new(name);
        sc.room_w = Some(self.room_w);
        sc.room_d = Some(self.room_d);
        sc.room_h = Some(self.room_h);
        sc.listener_dx = Some(self.listener_dx);
        sc.listener_dy = Some(self.listener_dy);
        sc.listener_z = Some(self.listener_z);
        sc.reverb_wet = Some(self.reverb_wet);
        sc
    }

    /// 音響ラボ用の合成 Cast（エンジン非依存。engine_volume_offsets は未登録キー→1.0）。
    pub fn to_cast(&self) -> Cast {
        Cast {
            name: "ラボ".into(),
            speaker_name: String::new(),
            engine_type: String::new(),
            pan: self.pan,
            distance: self.distance,
            volume: 1.0,
            params: HashMap::new(),
            height: Some(self.height),
            height_offset: 0.0,
        }
    }

    /// 台本に貼り付けられる @scene 行を生成する。
    pub fn scene_line(&self, scene_name: &str) -> String {
        format!(
            "@scene {} room_w={} room_d={} room_h={} listener_dx={} listener_dy={} listener_z={} reverb_wet={}",
            scene_name,
            self.room_w, self.room_d, self.room_h,
            self.listener_dx, self.listener_dy, self.listener_z,
            self.reverb_wet,
        )
    }
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p s2v-gui scene_line` → PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/scene_line.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): lab params with preset apply, scene-line generation (parser roundtrip tested)"
```

---

### Task 7: history.rs（試聴履歴・FIFO 50・A/B 選択）

**Files:**
- Create: `crates/s2v-gui/src/history.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod history;` 追加）

- [ ] **Step 1: 失敗するテストを書く**

```rust
use crate::scene_line::LabParams;
use std::collections::VecDeque;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub id: usize,
    pub params: LabParams,
    pub wav: PathBuf,
}

pub struct History {
    entries: VecDeque<HistoryEntry>,
    next_id: usize,
    cap: usize,
    pub sel_a: Option<usize>,
    pub sel_b: Option<usize>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(cap: usize) -> History {
        History::new(cap)
    }

    #[test]
    fn push_assigns_sequential_ids_and_caps_fifo() {
        let mut hist = h(3);
        for i in 0..5 {
            hist.push(LabParams::default(), PathBuf::from(format!("{i}.wav")));
        }
        let ids: Vec<usize> = hist.entries().map(|e| e.id).collect();
        assert_eq!(ids, vec![3, 4, 5], "古い順に追い出し・id は1始まり連番");
    }

    #[test]
    fn eviction_clears_dangling_selection() {
        let mut hist = h(2);
        let first = hist.push(LabParams::default(), "a.wav".into());
        hist.toggle_select(first);
        assert_eq!(hist.sel_a, Some(first));
        hist.push(LabParams::default(), "b.wav".into());
        hist.push(LabParams::default(), "c.wav".into()); // first が追い出される
        assert_eq!(hist.sel_a, None);
    }

    #[test]
    fn toggle_select_fills_a_then_b_then_replaces_b() {
        let mut hist = h(10);
        let a = hist.push(LabParams::default(), "a.wav".into());
        let b = hist.push(LabParams::default(), "b.wav".into());
        let c = hist.push(LabParams::default(), "c.wav".into());
        hist.toggle_select(a);
        hist.toggle_select(b);
        assert_eq!((hist.sel_a, hist.sel_b), (Some(a), Some(b)));
        hist.toggle_select(c); // 両方埋まり → B を置換
        assert_eq!((hist.sel_a, hist.sel_b), (Some(a), Some(c)));
        hist.toggle_select(a); // 再クリックで解除
        assert_eq!(hist.sel_a, None);
    }
}
```

- [ ] **Step 2: 失敗確認**

Run: `cargo test -p s2v-gui history` → コンパイルエラー

- [ ] **Step 3: 実装**

```rust
impl History {
    pub fn new(cap: usize) -> Self {
        Self { entries: VecDeque::new(), next_id: 1, cap, sel_a: None, sel_b: None }
    }

    /// 追加して採番した id を返す。あふれた分は WAV ファイルも削除する。
    pub fn push(&mut self, params: LabParams, wav: PathBuf) -> usize {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push_back(HistoryEntry { id, params, wav });
        while self.entries.len() > self.cap {
            if let Some(old) = self.entries.pop_front() {
                let _ = std::fs::remove_file(&old.wav);
                if self.sel_a == Some(old.id) { self.sel_a = None; }
                if self.sel_b == Some(old.id) { self.sel_b = None; }
            }
        }
        id
    }

    pub fn entries(&self) -> impl Iterator<Item = &HistoryEntry> {
        self.entries.iter()
    }

    pub fn get(&self, id: usize) -> Option<&HistoryEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// A→B の順に選択。選択済み id の再指定は解除。両方埋まっていたら B を置換。
    pub fn toggle_select(&mut self, id: usize) {
        if self.sel_a == Some(id) { self.sel_a = None; return; }
        if self.sel_b == Some(id) { self.sel_b = None; return; }
        if self.sel_a.is_none() { self.sel_a = Some(id); return; }
        self.sel_b = Some(id);
    }

    pub fn clear(&mut self) {
        for e in self.entries.drain(..) {
            let _ = std::fs::remove_file(&e.wav);
        }
        self.sel_a = None;
        self.sel_b = None;
    }
}
```

- [ ] **Step 4: テスト確認** `cargo test -p s2v-gui history` → PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/history.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): listening history model with FIFO cap and A/B selection"
```

---

### Task 8: script_model.rs（台本読込・BOM・行リスト・mtime 監視）

**Files:**
- Create: `crates/s2v-gui/src/script_model.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod script_model;` 追加）

- [ ] **Step 1: 失敗するテストを書く**

```rust
use s2v_core::{Cast, ParseWarning, Scene, SceneConfig, ScriptItem, ScriptParser};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// 行リスト1行分（プレビューに必要な情報を自己完結で持つ）。
#[derive(Debug, Clone)]
pub struct PreviewLine {
    /// 台本全体での speech 通し番号（1始まり = voice_NNNN と一致）
    pub no: usize,
    pub scene_name: String,
    pub cast_name: String,
    pub display_text: String,
    pub text: String,
    /// 行内臨時パラメータ適用済みの実効 Cast
    pub cast: Cast,
    pub scene_config: SceneConfig,
}

pub struct ScriptModel {
    pub path: PathBuf,
    pub lines: Vec<PreviewLine>,
    pub warnings: Vec<ParseWarning>,
    pub scenes: Vec<Scene>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    const SRC: &str = "@scene 一 room_size=0.3\n@cast\nA:話者:ノーマル,voicevox,pan=-30\n@script\nA:こんにちは\nA(pan=15):やあ\n誰か:無視される\n";

    #[test]
    fn strip_bom_removes_leading_bom_only() {
        assert_eq!(strip_bom("\u{feff}@scene"), "@scene");
        assert_eq!(strip_bom("@scene"), "@scene");
    }

    #[test]
    fn load_builds_lines_with_effective_cast_and_warnings() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("台本.txt");
        let mut f = std::fs::File::create(&path).unwrap();
        write!(f, "\u{feff}{SRC}").unwrap(); // BOM 付きでも読めること
        drop(f);
        let m = load(&path).unwrap();
        assert_eq!(m.lines.len(), 2);
        assert_eq!(m.lines[0].no, 1);
        assert_eq!(m.lines[0].scene_name, "一");
        assert_eq!(m.lines[0].cast.pan, -30.0);
        assert_eq!(m.lines[1].cast.pan, -15.0, "行内 pan=15 は加算オフセット");
        assert_eq!(m.lines[1].scene_config.room_size, Some(0.3));
        assert_eq!(m.warnings.len(), 1, "未定義キャスト警告");
    }

    #[test]
    fn watched_file_reports_change_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("台本.txt");
        std::fs::write(&path, "a").unwrap();
        let mut w = WatchedFile::with_interval(path.clone(), Duration::ZERO);
        assert!(!w.poll(), "初回登録時は変更扱いしない");
        // mtime を未来に更新して変更を模擬
        let f = std::fs::File::options().write(true).open(&path).unwrap();
        f.set_modified(SystemTime::now() + Duration::from_secs(2)).unwrap();
        drop(f);
        assert!(w.poll());
        assert!(!w.poll(), "同じ mtime では再通知しない");
    }
}
```

- [ ] **Step 2: 失敗確認**

`main.rs` に `mod script_model;` 追加後:
Run: `cargo test -p s2v-gui script_model` → コンパイルエラー

- [ ] **Step 3: 実装**

```rust
pub fn strip_bom(s: &str) -> &str {
    s.strip_prefix('\u{feff}').unwrap_or(s)
}

/// 台本を読み込み、行リスト＋警告を構築する。失敗時はメッセージを返す（呼び出し側で前回モデルを保持）。
pub fn load(path: &Path) -> Result<ScriptModel, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("台本を読み込めません: {e}"))?;
    let mut parser = ScriptParser::new();
    let scenes = parser
        .parse_str(strip_bom(&text))
        .map_err(|e| format!("台本の解析に失敗: {e}"))?;
    let warnings = parser.warnings().to_vec();

    let mut lines = Vec::new();
    let mut no = 0usize;
    for scene in &scenes {
        for item in &scene.items {
            let ScriptItem::Speech { cast_name, text, display_text, offset_params, scene_config } = item else {
                continue;
            };
            let Some(cast) = scene.casts.get(cast_name) else { continue; };
            no += 1;
            lines.push(PreviewLine {
                no,
                scene_name: scene.config.name.clone(),
                cast_name: cast_name.clone(),
                display_text: display_text.clone(),
                text: text.clone(),
                cast: cast.with_offsets(offset_params),
                scene_config: scene_config.clone(),
            });
        }
    }
    Ok(ScriptModel { path: path.to_path_buf(), lines, warnings, scenes })
}

/// mtime ポーリングによるファイル変更検知。
pub struct WatchedFile {
    path: PathBuf,
    last_mtime: Option<SystemTime>,
    last_check: Instant,
    interval: Duration,
}

impl WatchedFile {
    pub fn new(path: PathBuf) -> Self {
        Self::with_interval(path, Duration::from_millis(500))
    }

    pub fn with_interval(path: PathBuf, interval: Duration) -> Self {
        let last_mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok();
        Self { path, last_mtime, last_check: Instant::now(), interval }
    }

    /// 変更があれば true（500ms 間隔でのみ実チェック）。
    pub fn poll(&mut self) -> bool {
        if self.last_check.elapsed() < self.interval {
            return false;
        }
        self.last_check = Instant::now();
        let Ok(mtime) = std::fs::metadata(&self.path).and_then(|m| m.modified()) else {
            return false; // 一時的に消えている（エディタの保存中など）は無視
        };
        if self.last_mtime != Some(mtime) {
            self.last_mtime = Some(mtime);
            return true;
        }
        false
    }
}
```

注: `with_interval(.., Duration::ZERO)` のテストで `elapsed() < ZERO` は常に false → 毎回チェックされる。

- [ ] **Step 4: テスト確認** `cargo test -p s2v-gui script_model` → PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-gui/src/script_model.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): script model with BOM handling, effective casts, warnings, mtime watcher"
```

---

### Task 9: audio_play.rs と jobs.rs（再生・バックグラウンド処理）

**Files:**
- Create: `crates/s2v-gui/src/audio_play.rs`
- Create: `crates/s2v-gui/src/jobs.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod audio_play; mod jobs;` 追加）

- [ ] **Step 1: audio_play.rs**

```rust
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// rodio による単一ストリーム再生（同時再生は1つ。新しい再生で前を停止）。
pub struct Player {
    _stream: rodio::OutputStream,
    handle: rodio::OutputStreamHandle,
    sink: Option<rodio::Sink>,
}

impl Player {
    /// 出力デバイスが無い環境では None（UI 側で再生ボタンを無効化）。
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
```

- [ ] **Step 2: jobs.rs**

```rust
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
    tmp: tempfile::TempDir,
    lab_seq: std::sync::atomic::AtomicUsize,
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
        })
    }

    /// GUI 終了時に呼ぶ（自動起動したエンジンを停止）。
    pub fn shutdown(&self) {
        self.engines.shutdown_all();
    }

    /// 未起動ならエンジンを起動する（preview/run の前段で共用）。
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

    /// 台本1行のプレビュー（合成＋音響処理）。完了は JobMsg::PreviewReady。
    pub fn preview(&self, line: PreviewLine) {
        if self.busy_preview.swap(true, Ordering::SeqCst) {
            return; // 実行中は無視（UI 側でもボタン無効化）
        }
        let (tx, engines, processor, activated, busy) = (
            self.tx.clone(),
            Arc::clone(&self.engines),
            Arc::clone(&self.processor),
            Arc::clone(&self.activated),
            Arc::clone(&self.busy_preview),
        );
        let raw = self.tmp.path().join(format!("preview_{:04}_raw.wav", line.no));
        let out = self.tmp.path().join(format!("preview_{:04}.wav", line.no));
        self.rt.spawn(async move {
            let res: anyhow::Result<()> = async {
                let mut req = HashSet::new();
                req.insert(line.cast.engine_type.clone());
                Self::ensure_engines(&engines, &activated, req).await?;
                engines.synthesize(&line.text, &line.cast, &raw).await?;
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
                Ok(()) => tx.send(JobMsg::PreviewReady { line_no: line.no, wav: out, raw }),
                Err(e) => tx.send(JobMsg::PreviewFailed { line_no: line.no, error: format!("{e:#}") }),
            };
        });
    }

    /// 一括実行（CLI と同一出力）。進捗は RunPhase / RunProgress、完了は RunFinished。
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

                // ProduceEvent → JobMsg 変換（std mpsc を中継）
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

    /// 音響ラボ: 入力 WAV（任意 WAV or 行プレビューの raw）に音響処理を適用。
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
```

- [ ] **Step 3: ビルド確認**

`main.rs` に `mod audio_play; mod jobs;` を追加。
Run: `cargo build -p s2v-gui`
Expected: ビルド成功（この層は配線のみで分岐が薄いためユニットテストは置かず、Task 10/11 の手動スモークと既存ライブラリのテストで担保する）

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/audio_play.rs crates/s2v-gui/src/jobs.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): rodio player and background jobs (preview/run/lab) over tokio runtime"
```

---

### Task 10: タブ1「台本」UI と App 配線

**Files:**
- Create: `crates/s2v-gui/src/tab_script.rs`
- Modify: `crates/s2v-gui/src/app.rs`
- Modify: `crates/s2v-gui/src/main.rs`（`mod tab_script;` 追加、App::new に config 読込）

- [ ] **Step 1: tab_script.rs**

```rust
use std::path::PathBuf;

use crate::jobs::Jobs;
use crate::script_model::{self, ScriptModel, WatchedFile};

pub struct ScriptTab {
    pub model: Option<ScriptModel>,
    pub load_error: Option<String>,
    watcher: Option<WatchedFile>,
    pub auto_reload: bool,
    pub selected: Option<usize>,
    /// 直近プレビューの raw（音響ラボの「台本の行」音源）
    pub preview_raw: Option<(usize, PathBuf)>,
    pub preview_error: Option<String>,
    pub run_phase: String,
    pub run_progress: Option<(usize, usize)>,
    pub run_error: Option<String>,
    pub last_project_dir: Option<PathBuf>,
}

impl Default for ScriptTab {
    fn default() -> Self {
        Self {
            model: None,
            load_error: None,
            watcher: None,
            auto_reload: true,
            selected: None,
            preview_raw: None,
            preview_error: None,
            run_phase: String::new(),
            run_progress: None,
            run_error: None,
            last_project_dir: None,
        }
    }
}

impl ScriptTab {
    /// 前回開いた台本パスの保存先（exe と同じフォルダの s2v-gui.last）。
    fn last_path_file() -> Option<PathBuf> {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("s2v-gui.last")))
    }

    /// 起動時に前回の台本を自動で開く（無ければ何もしない）。
    pub fn restore_last(&mut self) {
        let Some(f) = Self::last_path_file() else { return };
        let Ok(s) = std::fs::read_to_string(&f) else { return };
        let p = PathBuf::from(s.trim());
        if p.exists() {
            self.open(p);
        }
    }

    pub fn open(&mut self, path: PathBuf) {
        match script_model::load(&path) {
            Ok(m) => {
                if let Some(f) = Self::last_path_file() {
                    let _ = std::fs::write(&f, path.to_string_lossy().as_bytes());
                }
                self.watcher = Some(WatchedFile::new(path));
                self.model = Some(m);
                self.load_error = None;
                self.selected = None;
            }
            Err(e) => self.load_error = Some(e), // 前回モデルは保持
        }
    }

    fn reload_if_changed(&mut self) {
        if !self.auto_reload {
            return;
        }
        let Some(w) = self.watcher.as_mut() else { return };
        if !w.poll() {
            return;
        }
        if let Some(m) = &self.model {
            let path = m.path.clone();
            match script_model::load(&path) {
                Ok(new_model) => {
                    self.model = Some(new_model);
                    self.load_error = None;
                    tracing::info!("台本を再読込しました: {}", path.display());
                }
                Err(e) => self.load_error = Some(e),
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, jobs: &Jobs) {
        self.reload_if_changed();

        ui.horizontal(|ui| {
            if ui.button("📂 台本を開く...").clicked() {
                if let Some(path) = rfd::FileDialog::new()
                    .add_filter("台本テキスト", &["txt"])
                    .pick_file()
                {
                    self.open(path);
                }
            }
            if let Some(m) = &self.model {
                ui.label(m.path.display().to_string());
            }
            ui.checkbox(&mut self.auto_reload, "保存時に自動再読込");
        });

        if let Some(e) = &self.load_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }
        if let Some(e) = &self.preview_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ 試聴失敗: {e}"));
        }

        let Some(model) = &self.model else {
            ui.label("台本ファイルを開いてください。");
            return;
        };

        for w in &model.warnings {
            ui.colored_label(
                egui::Color32::from_rgb(200, 130, 0),
                format!("⚠ {}行目: {}", w.line_no, w.message),
            );
        }

        ui.separator();

        let busy = jobs.busy_preview.load(std::sync::atomic::Ordering::SeqCst);
        let mut to_preview: Option<usize> = None;
        egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
            egui::Grid::new("lines").striped(true).num_columns(5).show(ui, |ui| {
                ui.strong("No");
                ui.strong("シーン");
                ui.strong("キャスト");
                ui.strong("台詞");
                ui.strong("");
                ui.end_row();
                for line in &model.lines {
                    let sel = self.selected == Some(line.no);
                    ui.label(line.no.to_string());
                    ui.label(&line.scene_name);
                    ui.label(&line.cast_name);
                    let text: String = line.display_text.chars().take(30).collect();
                    if ui.selectable_label(sel, text).clicked() {
                        self.selected = Some(line.no);
                    }
                    if ui.add_enabled(!busy, egui::Button::new("▶ 試聴")).clicked() {
                        self.selected = Some(line.no);
                        to_preview = Some(line.no);
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(no) = to_preview {
            if let Some(line) = model.lines.iter().find(|l| l.no == no) {
                self.preview_error = None;
                jobs.preview(line.clone());
            }
        }

        ui.separator();

        let running = jobs.busy_run.load(std::sync::atomic::Ordering::SeqCst);
        ui.horizontal(|ui| {
            if ui.add_enabled(!running, egui::Button::new("▶ 一括実行")).clicked() {
                self.run_error = None;
                self.run_progress = None;
                self.last_project_dir = None;
                jobs.run_all(model.path.clone());
            }
            if ui.add_enabled(running, egui::Button::new("⏹ キャンセル")).clicked() {
                jobs.cancel.store(true, std::sync::atomic::Ordering::SeqCst);
            }
            if running {
                ui.label(format!("実行中: {}", self.run_phase));
            }
        });
        if let Some((done, total)) = self.run_progress {
            ui.add(
                egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                    .text(format!("{done}/{total} 行")),
            );
        }
        if let Some(e) = &self.run_error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }
        if let Some(dir) = self.last_project_dir.clone() {
            ui.horizontal(|ui| {
                ui.label("✅ 完了");
                if ui.button("📁 出力フォルダを開く").clicked() {
                    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
                }
            });
        }
    }
}
```

- [ ] **Step 2: app.rs を配線**

`App` を拡張（フィールド追加・JobMsg ポンプ・設定読込・終了処理）。app.rs 全体を以下に置換:

```rust
use crate::audio_play::Player;
use crate::jobs::{JobMsg, Jobs};
use crate::logbuf::LogBuffer;
use crate::tab_script::ScriptTab;

#[derive(PartialEq, Clone, Copy)]
pub enum Tab {
    Script,
    Lab,
}

pub struct App {
    pub tab: Tab,
    pub log: LogBuffer,
    pub jobs: Option<Jobs>,
    pub jobs_error: Option<String>,
    pub player: Option<Player>,
    pub script: ScriptTab,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, log: LogBuffer) -> Self {
        // config.toml は CLI と同じ規約（exe と同じフォルダ）で解決
        let exe = std::env::current_exe().ok();
        let config_path = script2voice::resolve_config_path(None, exe.as_deref());
        let (jobs, jobs_error) = match s2v_core::Config::from_file(&config_path) {
            Ok(cfg) => match Jobs::new(cfg) {
                Ok(j) => (Some(j), None),
                Err(e) => (None, Some(format!("初期化失敗: {e}"))),
            },
            Err(e) => (None, Some(format!("config.toml を読めません ({}): {e}", config_path.display()))),
        };
        let mut script = ScriptTab::default();
        script.restore_last(); // 前回の台本パスを復元
        Self {
            tab: Tab::Script,
            log,
            jobs,
            jobs_error,
            player: Player::new(),
            script,
        }
    }

    fn pump_messages(&mut self) {
        let Some(jobs) = &self.jobs else { return };
        let msgs: Vec<JobMsg> = jobs.rx.try_iter().collect();
        for msg in msgs {
            match msg {
                JobMsg::PreviewReady { line_no, wav, raw } => {
                    self.script.preview_raw = Some((line_no, raw));
                    if let Some(p) = &mut self.player {
                        if let Err(e) = p.play(&wav) {
                            self.script.preview_error = Some(e.to_string());
                        }
                    }
                }
                JobMsg::PreviewFailed { error, .. } => {
                    self.script.preview_error = Some(error);
                }
                JobMsg::RunPhase(p) => self.script.run_phase = p,
                JobMsg::RunProgress { done, total } => {
                    self.script.run_progress = Some((done, total));
                }
                JobMsg::RunFinished { result } => match result {
                    Ok(dir) => self.script.last_project_dir = Some(dir),
                    Err(e) => self.script.run_error = Some(e),
                },
                JobMsg::LabReady { .. } | JobMsg::LabFailed { .. } => {
                    // Task 11 で音響ラボに配線
                }
            }
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.pump_messages();

        egui::TopBottomPanel::top("tabs").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, Tab::Script, "📜 台本");
                ui.selectable_value(&mut self.tab, Tab::Lab, "🎛 音響ラボ");
            });
        });
        egui::TopBottomPanel::bottom("log")
            .resizable(true)
            .default_height(120.0)
            .show(ctx, |ui| {
                ui.collapsing("実行ログ", |ui| {
                    egui::ScrollArea::vertical().stick_to_bottom(true).show(ui, |ui| {
                        for line in self.log.lines() {
                            ui.monospace(line);
                        }
                    });
                });
            });
        egui::CentralPanel::default().show(ctx, |ui| {
            if let Some(e) = &self.jobs_error {
                ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
                return;
            }
            match self.tab {
                Tab::Script => {
                    if let Some(jobs) = &self.jobs {
                        self.script.ui(ui, jobs);
                    }
                }
                Tab::Lab => {
                    ui.label("(音響ラボ: Task 11 で実装)");
                }
            }
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(250));
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if let Some(jobs) = &self.jobs {
            jobs.shutdown(); // 自動起動したエンジンを停止
        }
    }
}
```

注: `self.script.ui(ui, jobs)` で `&self.jobs` と `&mut self.script` の同時借用がコンパイルエラーになる場合は、`let jobs = self.jobs.as_ref();` を先に取り出す等で分離する（`Jobs` のメソッドはすべて `&self`）。それでも衝突する場合は `let jobs = self.jobs.take(); ... self.jobs = jobs;` パターンを使う。

`main.rs` に `mod tab_script;` を追加。

- [ ] **Step 3: ビルド・手動スモーク**

Run: `cargo run -p s2v-gui`（リポジトリ直下で実行すると config.toml は `target/debug` 探索になるため、確認時は `cargo build -p s2v-gui && copy config.toml target\debug\ && .\target\debug\s2v-gui.exe` で行う）
確認項目:
1. `scripts\音響テスト.txt` を開く → 行リストに12行表示、シーン名・キャスト名が正しい
2. 行の ▶試聴 → （VOICEVOX 自動起動後）音が鳴る。2回目は数秒で鳴る
3. 台本をメモ帳で編集・保存 → 行リストが自動更新される
4. 未定義キャスト行を足して保存 → ⚠警告が行番号付きで出る
5. ▶一括実行 → 進捗バーが進み、完了後「出力フォルダを開く」が機能する
6. GUI 終了 → VOICEVOX が停止する

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/tab_script.rs crates/s2v-gui/src/app.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): script tab (open/auto-reload/warnings/line preview/run with progress)"
```

---

### Task 11: タブ2「音響ラボ」UI と配線

**Files:**
- Create: `crates/s2v-gui/src/tab_lab.rs`
- Modify: `crates/s2v-gui/src/app.rs`（Lab 配線・JobMsg::Lab* 処理）
- Modify: `crates/s2v-gui/src/main.rs`（`mod tab_lab;` 追加）

- [ ] **Step 1: tab_lab.rs**

```rust
use std::path::PathBuf;

use crate::history::History;
use crate::jobs::Jobs;
use crate::presets::{self, Preset};
use crate::scene_line::LabParams;

pub enum LabSource {
    ScriptLine,
    WavFile,
}

pub struct LabTab {
    pub params: LabParams,
    pub presets: Vec<Preset>,
    pub preset_warning: Option<String>,
    pub selected_preset: usize,
    pub source: LabSource,
    pub source_wav: Option<PathBuf>,
    pub history: History,
    pub error: Option<String>,
}

impl LabTab {
    pub fn new() -> Self {
        // presets.toml は exe と同じフォルダ（config.toml と同じ規約）
        let path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("presets.toml")))
            .unwrap_or_else(|| PathBuf::from("presets.toml"));
        let (presets, preset_warning) = presets::load_presets(&path);
        Self {
            params: LabParams::default(),
            presets,
            preset_warning,
            selected_preset: 0,
            source: LabSource::WavFile,
            source_wav: None,
            history: History::new(50),
            error: None,
        }
    }

    /// 現在の音源入力（処理対象の WAV パス）。
    fn input(&self, preview_raw: &Option<(usize, PathBuf)>) -> Option<PathBuf> {
        match self.source {
            LabSource::ScriptLine => preview_raw.as_ref().map(|(_, p)| p.clone()),
            LabSource::WavFile => self.source_wav.clone(),
        }
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        jobs: &Jobs,
        preview_raw: &Option<(usize, PathBuf)>,
        player: &mut Option<crate::audio_play::Player>,
    ) {
        if let Some(w) = &self.preset_warning {
            ui.colored_label(egui::Color32::from_rgb(200, 130, 0), format!("⚠ {w}"));
        }
        if let Some(e) = &self.error {
            ui.colored_label(egui::Color32::RED, format!("⚠ {e}"));
        }

        // --- 音源選択 ---
        ui.horizontal(|ui| {
            ui.label("音源:");
            let has_line = preview_raw.is_some();
            if ui
                .add_enabled(
                    has_line,
                    egui::RadioButton::new(matches!(self.source, LabSource::ScriptLine),
                        match preview_raw {
                            Some((no, _)) => format!("台本の行 {no}（試聴済み）"),
                            None => "台本の行（先にタブ1で試聴）".to_string(),
                        }),
                )
                .clicked()
            {
                self.source = LabSource::ScriptLine;
            }
            if ui
                .radio(matches!(self.source, LabSource::WavFile), "WAVファイル")
                .clicked()
            {
                self.source = LabSource::WavFile;
            }
            if matches!(self.source, LabSource::WavFile) {
                if ui.button("選択...").clicked() {
                    if let Some(p) = rfd::FileDialog::new().add_filter("WAV", &["wav"]).pick_file() {
                        self.source_wav = Some(p);
                    }
                }
                if let Some(p) = &self.source_wav {
                    ui.label(p.file_name().and_then(|s| s.to_str()).unwrap_or("?"));
                }
            }
        });

        ui.separator();

        // --- プリセット ---
        ui.horizontal(|ui| {
            ui.label("プリセット:");
            egui::ComboBox::from_id_salt("preset")
                .selected_text(&self.presets[self.selected_preset].name)
                .show_ui(ui, |ui| {
                    for (i, p) in self.presets.iter().enumerate() {
                        ui.selectable_value(&mut self.selected_preset, i, &p.name);
                    }
                });
            if ui.button("適用").clicked() {
                let p = self.presets[self.selected_preset].clone();
                self.params.apply_preset(&p);
            }
        });

        // --- パラメータ ---
        egui::Grid::new("lab_params").num_columns(6).show(ui, |ui| {
            ui.label("部屋 幅[m]");
            ui.add(egui::Slider::new(&mut self.params.room_w, 2.0..=60.0));
            ui.label("奥行[m]");
            ui.add(egui::Slider::new(&mut self.params.room_d, 2.0..=80.0));
            ui.label("高さ[m]");
            ui.add(egui::Slider::new(&mut self.params.room_h, 2.0..=30.0));
            ui.end_row();
            ui.label("聴取 dx[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_dx, -20.0..=20.0));
            ui.label("dy[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_dy, -30.0..=30.0));
            ui.label("高さ z[m]");
            ui.add(egui::Slider::new(&mut self.params.listener_z, 0.2..=5.0));
            ui.end_row();
            ui.label("pan[°]");
            ui.add(egui::Slider::new(&mut self.params.pan, -90.0..=90.0));
            ui.label("距離[m]");
            ui.add(egui::Slider::new(&mut self.params.distance, 0.1..=30.0));
            ui.label("話者高さ[m]");
            ui.add(egui::Slider::new(&mut self.params.height, 0.0..=5.0));
            ui.end_row();
            ui.label("残響倍率");
            ui.add(egui::Slider::new(&mut self.params.reverb_wet, 0.0..=3.0));
            ui.end_row();
        });

        // --- 実行 ---
        let busy = jobs.busy_lab.load(std::sync::atomic::Ordering::SeqCst);
        ui.horizontal(|ui| {
            let input = self.input(preview_raw);
            if ui
                .add_enabled(!busy && input.is_some(), egui::Button::new("▶ 処理して試聴"))
                .clicked()
            {
                self.error = None;
                jobs.lab_process(input.unwrap(), self.params.clone());
            }
            if busy {
                ui.spinner();
            }
        });

        // --- @scene 行 ---
        let line = self.params.scene_line("シーン名");
        ui.horizontal(|ui| {
            ui.code(&line);
            if ui.button("📋 コピー").clicked() {
                ui.output_mut(|o| o.copied_text = line.clone());
            }
        });

        ui.separator();

        // --- 試聴履歴 ---
        ui.horizontal(|ui| {
            ui.strong("試聴履歴");
            let (a, b) = (self.history.sel_a, self.history.sel_b);
            if ui.add_enabled(a.is_some(), egui::Button::new("▶ A")).clicked() {
                if let (Some(id), Some(p)) = (a, player.as_mut()) {
                    if let Some(e) = self.history.get(id) {
                        let _ = p.play(&e.wav);
                    }
                }
            }
            if ui.add_enabled(b.is_some(), egui::Button::new("▶ B")).clicked() {
                if let (Some(id), Some(p)) = (b, player.as_mut()) {
                    if let Some(e) = self.history.get(id) {
                        let _ = p.play(&e.wav);
                    }
                }
            }
            if ui.button("クリア").clicked() {
                self.history.clear();
            }
        });

        let mut recall: Option<LabParams> = None;
        let mut play: Option<PathBuf> = None;
        let mut toggle: Option<usize> = None;
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            egui::Grid::new("history").striped(true).num_columns(5).show(ui, |ui| {
                for e in self.history.entries() {
                    let mark = if self.history.sel_a == Some(e.id) {
                        "A"
                    } else if self.history.sel_b == Some(e.id) {
                        "B"
                    } else {
                        ""
                    };
                    if ui.selectable_label(!mark.is_empty(), format!("#{} {}", e.id, mark)).clicked() {
                        toggle = Some(e.id);
                    }
                    ui.label(format!(
                        "部屋{}x{}x{} wet{} pan{} 距離{}",
                        e.params.room_w, e.params.room_d, e.params.room_h,
                        e.params.reverb_wet, e.params.pan, e.params.distance,
                    ));
                    if ui.button("▶").clicked() {
                        play = Some(e.wav.clone());
                    }
                    if ui.button("呼び戻す").clicked() {
                        recall = Some(e.params.clone());
                    }
                    if ui.button("💾 書き出し").clicked() {
                        if let Some(dest) = rfd::FileDialog::new()
                            .add_filter("WAV", &["wav"])
                            .set_file_name(format!("lab_{:04}.wav", e.id))
                            .save_file()
                        {
                            if let Err(err) = std::fs::copy(&e.wav, &dest) {
                                tracing::warn!("書き出し失敗: {err}");
                            }
                        }
                    }
                    ui.end_row();
                }
            });
        });
        if let Some(id) = toggle {
            self.history.toggle_select(id);
        }
        if let Some(p) = recall {
            self.params = p;
        }
        if let (Some(path), Some(pl)) = (play, player.as_mut()) {
            let _ = pl.play(&path);
        }
    }
}
```

- [ ] **Step 2: app.rs 配線**

`App` にフィールド追加: `pub lab: crate::tab_lab::LabTab,`（`new()` で `lab: crate::tab_lab::LabTab::new(),`）。
`pump_messages` の Lab アーム を置換:

```rust
                JobMsg::LabReady { wav, params } => {
                    self.lab.history.push(params, wav.clone());
                    if let Some(p) = &mut self.player {
                        let _ = p.play(&wav);
                    }
                }
                JobMsg::LabFailed { error } => self.lab.error = Some(error),
```

`Tab::Lab` アームを置換:

```rust
                Tab::Lab => {
                    if let Some(jobs) = &self.jobs {
                        let preview_raw = self.script.preview_raw.clone();
                        self.lab.ui(ui, jobs, &preview_raw, &mut self.player);
                    }
                }
```

`main.rs` に `mod tab_lab;` を追加。

- [ ] **Step 3: ビルド・全テスト・手動スモーク**

Run: `cargo test --workspace` → 全 PASS
Run: 手動（Task 10 と同じ起動方法）:
1. 音響ラボで `scripts\音響テスト\audio\voice_0001.wav` を音源に選び ▶処理して試聴 → 音が鳴る（エンジン起動なしで動く＝単独利用OK）
2. プリセット「2000席ホール」適用 → 再試聴 → 響きが変わる
3. 履歴に2件たまり、#1 と #2 をクリックで A/B 指定 → ▶A ▶B で交互再生できる
4. 「呼び戻す」でスライダーが戻る、💾書き出しで任意の場所に保存できる
5. @scene 行のコピー → メモ帳に貼り付けて文字列が正しい
6. タブ1で行を試聴後、ラボの音源「台本の行」を選択 → その声に処理がかかる

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-gui/src/tab_lab.rs crates/s2v-gui/src/app.rs crates/s2v-gui/src/main.rs
git commit -m "feat(gui): acoustic lab tab (presets, sliders, A/B history, scene-line copy, wav export)"
```

---

### Task 12: 仕上げ（presets.toml 同梱・マニュアル追記・最終確認）

**Files:**
- Create: `presets.toml`（リポジトリ直下、config.toml と並べる）
- Modify: `docs/manual.html`（GUI の節を「3. 基本操作」内に追加）

- [ ] **Step 1: presets.toml（サンプル同梱）**

```toml
# Script2Voice 音響ラボのプリセット定義。
# 実行ファイル（s2v-gui.exe）と同じフォルダに置く。
# すべての項目は省略可（省略項目は現在のスライダー値を維持）。

[[preset]]
name = "ライブハウス"
room_w = 15.0
room_d = 20.0
room_h = 6.0
listener_dy = -6.0
listener_z = 1.6
reverb_wet = 1.2

[[preset]]
name = "浴室（強い残響）"
room_w = 2.5
room_d = 3.0
room_h = 2.4
reverb_wet = 2.0
```

- [ ] **Step 2: マニュアル追記**

`docs/manual.html` の「3.3 出力されるファイル」の後（`<h2 id="script-format">` の前）に:

```html
<h3 id="gui">3.4 GUI（ランチャー＋音響ラボ）</h3>
<p>
  <code>s2v-gui.exe</code>（<code>cargo build --release -p s2v-gui</code> で生成）を起動すると、
  コマンドラインなしで台本の実行・確認ができます。<code>config.toml</code>・<code>presets.toml</code> は
  実行ファイルと同じフォルダに置いてください。
</p>
<ul>
  <li><strong>台本タブ</strong>: 台本を開くと行リストが表示され、保存のたびに自動再読込されます（編集はお使いのエディタで。UTF-8 保存）。
    行の「▶試聴」でその1行だけを合成・音響処理して確認できます。「▶一括実行」は CLI と同一の出力を生成します。</li>
  <li><strong>音響ラボタブ</strong>: 部屋寸法・聴取位置・残響倍率・話者位置をスライダーで変えながら、
    台本の行または任意の WAV に音響処理をかけて試聴できます。結果はパラメータとペアで履歴に残り、
    2件を選んで A/B 交互再生できます。決まった値は <code>@scene</code> 行としてコピーできます。</li>
</ul>
```

目次（`nav.toc` の「3. 基本操作」の `<ul>`）に `<li class="leaf"><a href="#gui">3.4 GUI（ランチャー＋音響ラボ）</a></li>` を追加。

- [ ] **Step 3: 最終確認**

Run: `cargo test --workspace` → 全 PASS
Run: `cargo build --release -p s2v-gui` → 成功
Run: `copy config.toml target\release\ ; copy presets.toml target\release\ ; .\target\release\s2v-gui.exe` → 起動・タブ操作・終了の通しスモーク

- [ ] **Step 4: コミット・Beads クローズ**

```bash
git add presets.toml docs/manual.html
git commit -m "feat(gui): bundle sample presets.toml and document GUI in manual"
bd close s2v-6p2
```
