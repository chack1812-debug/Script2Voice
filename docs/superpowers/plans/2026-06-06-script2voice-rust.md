# Script2Voice Rust版 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Python版Script2VoiceをRustで再構築し、台本テキストから空間音響処理済みの音声・字幕・タイムラインを生成するCLIツールを作る。

**Architecture:** Cargo Workspaceに4クレート（s2v-core/s2v-engines/s2v-audio/s2v-export）＋ルートバイナリ（Producer+CLI）。Engine traitでHTTP音声合成エンジン3種を抽象化し、tokio非同期+rayon並列で合成・音響処理を実行する。

**Tech Stack:** Rust edition 2021, tokio, reqwest, hound, rubato, realfft, rayon, clap, serde/toml, tracing, anyhow, thiserror, async-trait, rand

---

## ファイルマップ

```
Cargo.toml                              workspace定義
config.toml                             デフォルト設定
src/lib.rs                              Producer構造体
src/main.rs                             CLIエントリポイント

crates/s2v-core/
  Cargo.toml
  src/lib.rs
  src/config.rs                         Config構造体 (serde/toml)
  src/cast.rs                           Cast構造体 + with_offsets()
  src/parser.rs                         ScriptParser (全セクション)
  src/timeline.rs                       TimelineProcessor + TimelineEvent
  tests/parser_tests.rs

crates/s2v-engines/
  Cargo.toml
  src/lib.rs                            Engine trait + EngineManager
  src/http_engine.rs                    VOICEVOX/AivisSpeech共通HTTP実装
  src/xtts.rs                           XTTS HTTP実装
  tests/engine_tests.rs

crates/s2v-audio/
  Cargo.toml
  src/lib.rs
  src/resample.rs                       rubato wrapper
  src/reverb.rs                         IR生成 + realfft畳み込み
  src/processor.rs                      AudioProcessor (full pipeline)
  tests/audio_tests.rs

crates/s2v-export/
  Cargo.toml
  src/lib.rs
  src/srt.rs                            SRT字幕生成
  src/fcpxml.rs                         FCPXML 1.8生成
  src/mix.rs                            WAVミックス (hound)
  tests/export_tests.rs
```

---

## Task 1: Workspace スキャフォールド

**Files:**
- Create: `Cargo.toml`
- Create: `src/main.rs`, `src/lib.rs`
- Create: `crates/s2v-core/Cargo.toml`, `crates/s2v-core/src/lib.rs`
- Create: `crates/s2v-engines/Cargo.toml`, `crates/s2v-engines/src/lib.rs`
- Create: `crates/s2v-audio/Cargo.toml`, `crates/s2v-audio/src/lib.rs`
- Create: `crates/s2v-export/Cargo.toml`, `crates/s2v-export/src/lib.rs`

- [ ] **Step 1: workspace Cargo.toml を作成**

```toml
# Cargo.toml
[workspace]
members = [".", "crates/s2v-core", "crates/s2v-engines", "crates/s2v-audio", "crates/s2v-export"]
resolver = "2"

[workspace.dependencies]
anyhow = "1"
thiserror = "2"
serde = { version = "1", features = ["derive"] }
tokio = { version = "1", features = ["full"] }
tracing = "0.1"

[package]
name = "script2voice"
version = "0.1.0"
edition = "2021"

[[bin]]
name = "script2voice"
path = "src/main.rs"

[lib]
path = "src/lib.rs"

[dependencies]
s2v-core    = { path = "crates/s2v-core" }
s2v-engines = { path = "crates/s2v-engines" }
s2v-audio   = { path = "crates/s2v-audio" }
s2v-export  = { path = "crates/s2v-export" }
anyhow.workspace = true
tokio.workspace = true
tracing.workspace = true
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt"] }
clap = { version = "4", features = ["derive"] }
rayon = "1"
```

- [ ] **Step 2: 各クレートの Cargo.toml を作成**

```toml
# crates/s2v-core/Cargo.toml
[package]
name = "s2v-core"
version = "0.1.0"
edition = "2021"

[dependencies]
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
toml = "0.8"
ordered-float = { version = "4", features = ["serde"] }
```

```toml
# crates/s2v-engines/Cargo.toml
[package]
name = "s2v-engines"
version = "0.1.0"
edition = "2021"

[dependencies]
s2v-core = { path = "../s2v-core" }
anyhow.workspace = true
thiserror.workspace = true
serde.workspace = true
tokio.workspace = true
reqwest = { version = "0.12", features = ["json"] }
async-trait = "0.1"
tracing.workspace = true

[dev-dependencies]
wiremock = "0.6"
tokio = { version = "1", features = ["full"] }
```

```toml
# crates/s2v-audio/Cargo.toml
[package]
name = "s2v-audio"
version = "0.1.0"
edition = "2021"

[dependencies]
s2v-core = { path = "../s2v-core" }
anyhow.workspace = true
thiserror.workspace = true
hound = "3"
rubato = "0.15"
realfft = "3"
rayon = "1"
rand = "0.8"
rand_distr = "0.4"
ordered-float = "4"
tracing.workspace = true
```

```toml
# crates/s2v-export/Cargo.toml
[package]
name = "s2v-export"
version = "0.1.0"
edition = "2021"

[dependencies]
s2v-core  = { path = "../s2v-core" }
s2v-audio = { path = "../s2v-audio" }
anyhow.workspace = true
thiserror.workspace = true
hound = "3"
tracing.workspace = true
```

- [ ] **Step 3: スタブ lib.rs を作成**

各クレートの `src/lib.rs` に空ファイル、`src/main.rs` に最小実装を置く：

```rust
// src/main.rs
fn main() { println!("script2voice"); }
```

```rust
// src/lib.rs (root)
// Producer は後のタスクで実装
```

各 `crates/*/src/lib.rs` は空ファイルで OK。

- [ ] **Step 4: ビルド確認**

```
cargo build
```
Expected: `Finished` (warning は無視)

- [ ] **Step 5: コミット**

```
git add Cargo.toml src/ crates/
git commit -m "chore: scaffold cargo workspace with 4 crates"
```

---

## Task 2: s2v-core — Config

**Files:**
- Create: `crates/s2v-core/src/config.rs`
- Create: `config.toml`
- Modify: `crates/s2v-core/src/lib.rs`

- [ ] **Step 1: config.rs を作成**

```rust
// crates/s2v-core/src/config.rs
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Debug, Deserialize, Clone)]
pub struct Config {
    pub voicevox: EngineConfig,
    pub aivis: EngineConfig,
    pub xtts: EngineConfig,
    pub audio: AudioConfig,
    pub concurrency: ConcurrencyConfig,
    pub bgm: BgmConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct EngineConfig {
    pub url: String,
}

#[derive(Debug, Deserialize, Clone)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub microphone_spacing: f64,
    pub sound_speed: f64,
    pub air_absorption_coeff: f64,
    pub room_size: f64,
    pub reverb_wet: f64,
    pub reference_dist: f64,
    pub reference_gain_db: f64,
    pub max_gain_db: f64,
    pub mic_directivity: f64,
    pub mic_angle: f64,
    pub engine_volume_offsets: HashMap<String, f64>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ConcurrencyConfig {
    pub voicevox: usize,
    pub aivis: usize,
    pub xtts: usize,
    pub audio_process: usize,
}

#[derive(Debug, Deserialize, Clone)]
pub struct BgmConfig {
    pub crossfade_s: f64,
    pub se_fade_out_s: f64,
}

impl Config {
    pub fn load(path: &std::path::Path) -> anyhow::Result<Self> {
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }
}
```

- [ ] **Step 2: config.toml を作成**

```toml
# config.toml
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
room_size = 0.1
reverb_wet = 0.7
reference_dist = 1.0
reference_gain_db = -5.0
max_gain_db = -1.0
mic_directivity = 0.5
mic_angle = 45.0

[audio.engine_volume_offsets]
voicevox = 1.2
aivis = 0.9
xtts = 1.0

[concurrency]
voicevox = 3
aivis = 3
xtts = 2
audio_process = 0

[bgm]
crossfade_s = 3.0
se_fade_out_s = 0.05
```

- [ ] **Step 3: lib.rs に pub mod 追加**

```rust
// crates/s2v-core/src/lib.rs
pub mod config;
pub use config::Config;
```

- [ ] **Step 4: テスト作成・実行**

```rust
// crates/s2v-core/src/config.rs の末尾に追加
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_load_config() {
        let cfg = Config::load(std::path::Path::new("../../config.toml")).unwrap();
        assert_eq!(cfg.audio.sample_rate, 48000);
        assert_eq!(cfg.concurrency.voicevox, 3);
        assert!((cfg.audio.engine_volume_offsets["voicevox"] - 1.2).abs() < 1e-6);
    }
}
```

```
cargo test -p s2v-core test_load_config
```
Expected: PASS

- [ ] **Step 5: コミット**

```
git add crates/s2v-core/ config.toml
git commit -m "feat(core): add Config struct with toml loading"
```

---

## Task 3: s2v-core — Cast

**Files:**
- Create: `crates/s2v-core/src/cast.rs`
- Modify: `crates/s2v-core/src/lib.rs`

- [ ] **Step 1: テスト作成**

```rust
// crates/s2v-core/tests/cast_tests.rs
use s2v_core::cast::Cast;
use std::collections::HashMap;

#[test]
fn test_with_offsets_pan() {
    let mut params = HashMap::new();
    params.insert("style".to_string(), serde_json::Value::String("normal".into()));
    let cast = Cast {
        name: "A".into(), speaker_name: "sp".into(), engine_type: "xtts".into(),
        pan: 10.0, distance: 1.5, volume: 1.0, params,
    };
    let mut offsets = HashMap::new();
    offsets.insert("pan".to_string(), 5.0);
    let effective = cast.with_offsets(&offsets);
    assert!((effective.pan - 15.0).abs() < 1e-9);
    assert!((effective.distance - 1.5).abs() < 1e-9);
}

#[test]
fn test_with_offsets_engine_param() {
    let mut params = HashMap::new();
    params.insert("speedScale".to_string(), serde_json::Value::from(1.0_f64));
    let cast = Cast {
        name: "A".into(), speaker_name: "sp".into(), engine_type: "voicevox".into(),
        pan: 0.0, distance: 1.0, volume: 1.0, params,
    };
    let mut offsets = HashMap::new();
    offsets.insert("speedScale".to_string(), 0.2);
    let effective = cast.with_offsets(&offsets);
    let speed = effective.params["speedScale"].as_f64().unwrap();
    assert!((speed - 1.2).abs() < 1e-9);
}
```

- [ ] **Step 2: テスト失敗を確認**

```
cargo test -p s2v-core --test cast_tests 2>&1 | head -5
```
Expected: compile error (Cast未定義)

- [ ] **Step 3: cast.rs 実装**

```rust
// crates/s2v-core/src/cast.rs
use std::collections::HashMap;
use serde_json::Value;

#[derive(Debug, Clone)]
pub struct Cast {
    pub name: String,
    pub speaker_name: String,
    pub engine_type: String,
    pub pan: f64,
    pub distance: f64,
    pub volume: f64,
    pub params: HashMap<String, Value>,
}

// エンジンパラメータの中立値（オフセット加算の基準）
const ENGINE_PARAM_DEFAULTS: &[(&str, f64)] = &[
    ("speedScale", 1.0), ("pitchScale", 0.0), ("intonationScale", 1.0),
    ("volumeScale", 1.0), ("tempoDynamicsScale", 1.0),
    ("speed", 1.0), ("temperature", 0.0), ("pitch", 0.0),
];

impl Cast {
    /// 臨時パラメータを適用した新 Cast を返す。数値は加算、style は上書き。
    pub fn with_offsets(&self, offsets: &HashMap<String, f64>) -> Self {
        if offsets.is_empty() { return self.clone(); }
        let mut new_params = self.params.clone();
        let mut new_pan = self.pan;
        let mut new_distance = self.distance;
        let mut new_volume = self.volume;

        for (k, &v) in offsets {
            if let Some(&(_, default)) = ENGINE_PARAM_DEFAULTS.iter().find(|&&(n, _)| n == k) {
                let base = self.params.get(k).and_then(|v| v.as_f64()).unwrap_or(default);
                new_params.insert(k.clone(), Value::from(base + v));
            } else {
                match k.as_str() {
                    "pan"      => new_pan      += v,
                    "distance" => new_distance += v,
                    "volume"   => new_volume   += v,
                    _          => { new_params.insert(k.clone(), Value::from(v)); }
                }
            }
        }
        Cast { name: self.name.clone(), speaker_name: self.speaker_name.clone(),
               engine_type: self.engine_type.clone(),
               pan: new_pan, distance: new_distance, volume: new_volume,
               params: new_params }
    }
}
```

- [ ] **Step 4: lib.rs に追加。serde_json 依存も追加**

`crates/s2v-core/Cargo.toml` に `serde_json = "1"` を追加。

```rust
// crates/s2v-core/src/lib.rs
pub mod config;
pub mod cast;
pub use config::Config;
pub use cast::Cast;
```

- [ ] **Step 5: テスト PASS 確認**

```
cargo test -p s2v-core --test cast_tests
```
Expected: 2 tests PASS

- [ ] **Step 6: コミット**

```
git add crates/s2v-core/
git commit -m "feat(core): add Cast struct with with_offsets()"
```

---

## Task 4: s2v-core — Parser 型定義 + セクション検出

**Files:**
- Create: `crates/s2v-core/src/parser.rs`
- Modify: `crates/s2v-core/src/lib.rs`

- [ ] **Step 1: parser.rs に型定義を作成**

```rust
// crates/s2v-core/src/parser.rs
use std::collections::HashMap;
use crate::cast::Cast;

#[derive(Debug, Clone)]
pub struct SceneConfig {
    pub room_size: Option<f64>,
    pub reverb_wet: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct Scene {
    pub name: String,
    pub config: SceneConfig,
    pub items: Vec<ScriptItem>,
}

#[derive(Debug, Clone)]
pub enum ScriptItem {
    Speech {
        cast_name: String,
        text: String,
        display_text: String,
        offset_params: HashMap<String, f64>,
        scene_config: SceneConfig,
    },
    Command(ScriptCommand),
}

#[derive(Debug, Clone)]
pub enum ScriptCommand {
    Pause(f64),
    Paragraph,
    BgmStart(String),
    BgmStop,
    Se(String),
    Parallel(usize),
}

#[derive(Debug, Clone)]
pub struct PauseConfig {
    pub sentence_ms: f64,
    pub cast_ms: f64,
    pub paragraph_ms: f64,
}

impl Default for PauseConfig {
    fn default() -> Self {
        Self { sentence_ms: 500.0, cast_ms: 300.0, paragraph_ms: 1500.0 }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AssetConfig {
    pub bgm_dir: String,
    pub se_dir: String,
}

#[derive(Debug, Default)]
pub struct ScriptParser {
    pub casts: HashMap<String, Cast>,
    pub pause_config: PauseConfig,
    pub asset_config: AssetConfig,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Section { None, Scene, Pause, Asset, Cast, Script }

impl ScriptParser {
    pub fn new() -> Self { Self::default() }
}
```

- [ ] **Step 2: lib.rs に pub mod parser 追加**

```rust
// crates/s2v-core/src/lib.rs
pub mod config;
pub mod cast;
pub mod parser;
pub use config::Config;
pub use cast::Cast;
pub use parser::{ScriptParser, Scene, ScriptItem, ScriptCommand, PauseConfig, AssetConfig};
```

- [ ] **Step 3: ビルド確認**

```
cargo build -p s2v-core
```
Expected: PASS

- [ ] **Step 4: コミット**

```
git add crates/s2v-core/
git commit -m "feat(core): add parser types (Scene, ScriptItem, ScriptCommand)"
```

---

## Task 5: s2v-core — Parser 実装

**Files:**
- Modify: `crates/s2v-core/src/parser.rs`
- Create: `crates/s2v-core/tests/parser_tests.rs`

- [ ] **Step 1: テスト作成**

```rust
// crates/s2v-core/tests/parser_tests.rs
use s2v_core::parser::{ScriptParser, ScriptItem, ScriptCommand};

const SAMPLE_SCRIPT: &str = r#"
@pause
sentence 400
cast 200
paragraph 1000

@asset
bgm_dir = C:/audio/bgm
se_dir = C:/audio/se

@cast
ナレーター:四国めたん:ノーマル,voicevox,pan=-30.0,distance=2.0
キャラA:Zundamon:あまあま,aivis

@scene 室内
@script
ナレーター:これはテストです。
キャラA:こんにちは。
#pause 500
#paragraph
2
ナレーター:同時発声A
キャラA:同時発声B
"#;

#[test]
fn test_pause_config() {
    let mut p = ScriptParser::new();
    p.parse_str(SAMPLE_SCRIPT).unwrap();
    assert!((p.pause_config.sentence_ms - 400.0).abs() < 1e-9);
    assert!((p.pause_config.cast_ms    - 200.0).abs() < 1e-9);
    assert!((p.pause_config.paragraph_ms - 1000.0).abs() < 1e-9);
}

#[test]
fn test_asset_config() {
    let mut p = ScriptParser::new();
    p.parse_str(SAMPLE_SCRIPT).unwrap();
    assert_eq!(p.asset_config.bgm_dir, "C:/audio/bgm");
    assert_eq!(p.asset_config.se_dir,  "C:/audio/se");
}

#[test]
fn test_cast_parsing() {
    let mut p = ScriptParser::new();
    p.parse_str(SAMPLE_SCRIPT).unwrap();
    let n = p.casts.get("ナレーター").unwrap();
    assert_eq!(n.speaker_name, "四国めたん");
    assert_eq!(n.engine_type,  "voicevox");
    assert!((n.pan - -30.0).abs() < 1e-9);
    assert!((n.distance - 2.0).abs() < 1e-9);
}

#[test]
fn test_scene_and_speech() {
    let mut p = ScriptParser::new();
    let scenes = p.parse_str(SAMPLE_SCRIPT).unwrap();
    assert_eq!(scenes.len(), 1);
    assert_eq!(scenes[0].name, "室内");
    let speeches: Vec<_> = scenes[0].items.iter().filter(|i| matches!(i, ScriptItem::Speech {..})).collect();
    assert_eq!(speeches.len(), 4); // 2通常 + 2同時
}

#[test]
fn test_parallel_command() {
    let mut p = ScriptParser::new();
    let scenes = p.parse_str(SAMPLE_SCRIPT).unwrap();
    let parallel = scenes[0].items.iter().find(|i| {
        matches!(i, ScriptItem::Command(ScriptCommand::Parallel(2)))
    });
    assert!(parallel.is_some());
}

#[test]
fn test_rubi_expansion() {
    let mut p = ScriptParser::new();
    let script = "@cast\nA:sp:def,xtts\n@scene s\n@script\nA:'日本語:にほんご'の文章\n";
    let scenes = p.parse_str(script).unwrap();
    if let ScriptItem::Speech { text, display_text, .. } = &scenes[0].items[0] {
        assert_eq!(text, "にほんごの文章");
        assert_eq!(display_text, "日本語の文章");
    } else { panic!("not speech"); }
}
```

- [ ] **Step 2: テスト失敗確認**

```
cargo test -p s2v-core --test parser_tests 2>&1 | head -5
```
Expected: compile error

- [ ] **Step 3: parser.rs に parse_str / 全セクションパース実装**

```rust
// crates/s2v-core/src/parser.rs (impl ScriptParser に追加)

use serde_json::Value;
use std::path::Path;

impl ScriptParser {
    pub fn parse_file(&mut self, path: &Path) -> anyhow::Result<Vec<Scene>> {
        let text = std::fs::read_to_string(path)?;
        self.parse_str(&text)
    }

    pub fn parse_str(&mut self, text: &str) -> anyhow::Result<Vec<Scene>> {
        let mut section = Section::None;
        let mut scenes: Vec<Scene> = Vec::new();
        let mut current_scene: Option<Scene> = None;

        for line in text.lines() {
            let line = line.trim();
            if line.is_empty() { continue; }

            if line.starts_with('@') {
                // シーン切り替え時に保存
                if line.starts_with("@scene") {
                    if let Some(s) = current_scene.take() { scenes.push(s); }
                    current_scene = Some(Self::parse_scene_header(line));
                }
                section = match line {
                    l if l.starts_with("@scene")  => Section::Script, // @scene 後は @script 扱い
                    l if l.starts_with("@pause")  => Section::Pause,
                    l if l.starts_with("@asset")  => Section::Asset,
                    l if l.starts_with("@cast")   => Section::Cast,
                    l if l.starts_with("@script") => Section::Script,
                    _ => section,
                };
                // @scene 行自体は scene ヘッダーとして処理済み
                if line.starts_with("@scene") { section = Section::Scene; }
                continue;
            }

            // @scene 行の次からは @script として扱う（シーン内の最初のセクション指定まで）
            match section {
                Section::Pause  => self.parse_pause_line(line),
                Section::Asset  => self.parse_asset_line(line),
                Section::Cast   => self.parse_cast_line(line),
                Section::Script => {
                    if let Some(scene) = current_scene.as_mut() {
                        if let Some(item) = self.parse_script_line(line) {
                            let cfg = scene.config.clone();
                            let item = Self::attach_scene_config(item, &cfg);
                            scene.items.push(item);
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(s) = current_scene { scenes.push(s); }
        Ok(scenes)
    }

    fn parse_scene_header(line: &str) -> Scene {
        let rest = line.strip_prefix("@scene").unwrap_or("").trim();
        let tokens: Vec<&str> = rest.split_whitespace().collect();
        let mut name_tokens = Vec::new();
        let mut room_size = None;
        let mut reverb_wet = None;
        for t in &tokens {
            if t.starts_with("room_size=") {
                room_size = t["room_size=".len()..].parse().ok();
            } else if t.starts_with("reverb_wet=") {
                reverb_wet = t["reverb_wet=".len()..].parse().ok();
            } else {
                name_tokens.push(*t);
            }
        }
        Scene { name: name_tokens.join(" "), config: SceneConfig { room_size, reverb_wet }, items: Vec::new() }
    }

    fn parse_pause_line(&mut self, line: &str) {
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() < 2 { return; }
        let val: f64 = match parts[1].trim().parse() { Ok(v) => v, Err(_) => return };
        match parts[0] {
            "sentence" | "sentens" => self.pause_config.sentence_ms   = val,
            "cast"                 => self.pause_config.cast_ms        = val,
            "paragraph"            => self.pause_config.paragraph_ms   = val,
            _ => {}
        }
    }

    fn parse_asset_line(&mut self, line: &str) {
        if let Some((k, v)) = line.split_once('=') {
            match k.trim() {
                "bgm_dir" => self.asset_config.bgm_dir = v.trim().to_string(),
                "se_dir"  => self.asset_config.se_dir  = v.trim().to_string(),
                _ => {}
            }
        }
    }

    fn parse_cast_line(&mut self, line: &str) {
        // format: 役名:話者名:スタイル,エンジン[,params...]
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 { return; }
        let name = parts[0].trim().to_string();
        let speaker_name = parts[1].trim().to_string();
        let remain = parts[2];

        let sub: Vec<&str> = remain.splitn(3, ',').collect();
        let style = sub[0].trim().to_string();
        let engine_type = sub.get(1).map(|s| s.trim().to_lowercase()).unwrap_or_default();
        let params_str = sub.get(2).copied().unwrap_or("").to_string();

        let mut raw = Self::extract_kv_params(&params_str);
        let pan      = raw.remove("pan").unwrap_or(0.0);
        let distance = raw.remove("distance").unwrap_or(1.0);
        let volume   = raw.remove("volume").unwrap_or(1.0);

        let mut params: HashMap<String, Value> = raw.into_iter().map(|(k, v)| (k, Value::from(v))).collect();
        params.insert("style".to_string(), Value::String(style));

        self.casts.insert(name.clone(), Cast { name, speaker_name, engine_type, pan, distance, volume, params });
    }

    fn parse_script_line(&self, line: &str) -> Option<ScriptItem> {
        // 数字のみ → Parallel
        if line.chars().all(|c| c.is_ascii_digit()) {
            let n: usize = line.parse().ok()?;
            return Some(ScriptItem::Command(ScriptCommand::Parallel(n)));
        }
        // # コマンド
        if line.starts_with('#') {
            let rest = &line[1..];
            if rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t') { return None; }
            let parts: Vec<&str> = rest.splitn(2, char::is_whitespace).collect();
            let cmd = match parts[0] {
                "pause"     => ScriptCommand::Pause(parts.get(1).and_then(|s| s.trim().parse().ok()).unwrap_or(0.0)),
                "paragraph" => ScriptCommand::Paragraph,
                "bgm_start" => ScriptCommand::BgmStart(parts.get(1).unwrap_or(&"").trim().to_string()),
                "bgm_stop"  => ScriptCommand::BgmStop,
                "se"        => ScriptCommand::Se(parts.get(1).unwrap_or(&"").trim().to_string()),
                _ => return None,
            };
            return Some(ScriptItem::Command(cmd));
        }
        // 台詞行
        let sep = if line.contains(':') { ':' } else if line.contains('：') { '：' } else { return None; };
        let (name_part, raw_text) = line.split_once(sep)?;
        let name_part = name_part.trim();

        // 役名(臨時パラメータ)
        let (role, offset_params) = if let Some(paren_start) = name_part.find('(') {
            let role = &name_part[..paren_start];
            let params_str = name_part[paren_start+1..].trim_end_matches(')');
            (role.trim(), Self::extract_kv_params(params_str))
        } else {
            (name_part, HashMap::new())
        };

        if !self.casts.contains_key(role) { return None; }

        let (text, display_text) = Self::expand_rubi(raw_text.trim());
        Some(ScriptItem::Speech {
            cast_name: role.to_string(),
            text,
            display_text,
            offset_params,
            scene_config: SceneConfig { room_size: None, reverb_wet: None },
        })
    }

    fn attach_scene_config(item: ScriptItem, cfg: &SceneConfig) -> ScriptItem {
        if let ScriptItem::Speech { cast_name, text, display_text, offset_params, .. } = item {
            ScriptItem::Speech { cast_name, text, display_text, offset_params, scene_config: cfg.clone() }
        } else { item }
    }

    /// 'word:reading' 形式のルビを展開し (synthesis_text, display_text) を返す
    fn expand_rubi(text: &str) -> (String, String) {
        let mut synthesis = text.to_string();
        let mut display   = text.to_string();
        let re_pattern = "'([^':：]+?)[:：]([^':：]+?)'";
        // 簡易パース（regex クレートを避けてコンパイル時間を短縮）
        let mut s_out = String::new();
        let mut d_out = String::new();
        let bytes = text.as_bytes();
        let mut i = 0;
        while i < text.len() {
            if bytes[i] == b'\'' {
                if let Some(end) = text[i+1..].find('\'') {
                    let inner = &text[i+1..i+1+end];
                    let sep_pos = inner.find(':').or_else(|| inner.find('：'));
                    if let Some(sep) = sep_pos {
                        let word    = &inner[..sep];
                        let reading = &inner[sep+1..];
                        s_out.push_str(reading);
                        d_out.push_str(word);
                        i += 1 + end + 1;
                        continue;
                    }
                }
            }
            let ch = &text[i..i+text[i..].chars().next().map(|c| c.len_utf8()).unwrap_or(1)];
            s_out.push_str(ch);
            d_out.push_str(ch);
            i += ch.len();
        }
        (s_out, d_out)
    }

    pub fn extract_kv_params(s: &str) -> HashMap<String, f64> {
        let mut map = HashMap::new();
        for part in s.split(',') {
            if let Some((k, v)) = part.split_once('=') {
                if let Ok(val) = v.trim().parse::<f64>() {
                    map.insert(k.trim().to_string(), val);
                }
            }
        }
        map
    }
}
```

- [ ] **Step 4: テスト PASS 確認**

```
cargo test -p s2v-core --test parser_tests
```
Expected: 6 tests PASS

- [ ] **Step 5: コミット**

```
git add crates/s2v-core/
git commit -m "feat(core): implement ScriptParser with all sections"
```

---

## Task 6: s2v-core — TimelineProcessor

**Files:**
- Create: `crates/s2v-core/src/timeline.rs`
- Modify: `crates/s2v-core/src/lib.rs`

- [ ] **Step 1: テスト作成**

```rust
// crates/s2v-core/tests/timeline_tests.rs
use s2v_core::timeline::{TimelineProcessor, TimelineEvent, EventType};
use s2v_core::parser::PauseConfig;

#[test]
fn test_register_and_advance() {
    let pause = PauseConfig { sentence_ms: 300.0, cast_ms: 500.0, paragraph_ms: 1500.0 };
    let mut tl = TimelineProcessor::new(pause);
    tl.register_audio("a.wav".into(), 1000.0, 0.0, "text".into(), "disp".into(), "A".into());
    tl.advance_after_speech(1000.0, 300.0);
    assert!((tl.current_ms - 1300.0).abs() < 1e-9);
}

#[test]
fn test_bgm_registration() {
    let mut tl = TimelineProcessor::new(PauseConfig::default());
    tl.register_bgm("bgm.wav".into());
    let events = tl.events();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type, EventType::BgmStart);
}
```

- [ ] **Step 2: timeline.rs 実装**

```rust
// crates/s2v-core/src/timeline.rs
use std::path::PathBuf;
use crate::parser::PauseConfig;

#[derive(Debug, Clone, PartialEq)]
pub enum EventType { Audio, BgmStart, BgmStop, Se }

#[derive(Debug, Clone)]
pub struct TimelineEvent {
    pub event_type: EventType,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub path: Option<PathBuf>,
    pub text: Option<String>,
    pub display_text: Option<String>,
    pub cast: Option<String>,
}

pub struct TimelineProcessor {
    pub current_ms: f64,
    events_inner: Vec<TimelineEvent>,
    sentence_pause_ms: f64,
    cast_pause_ms: f64,
    paragraph_pause_ms: f64,
}

impl TimelineProcessor {
    pub fn new(cfg: PauseConfig) -> Self {
        Self { current_ms: 0.0, events_inner: Vec::new(),
               sentence_pause_ms: cfg.sentence_ms,
               cast_pause_ms: cfg.cast_ms,
               paragraph_pause_ms: cfg.paragraph_ms }
    }

    pub fn register_audio(&mut self, path: PathBuf, duration_ms: f64, start_ms: f64,
                          text: String, display_text: String, cast: String) {
        self.events_inner.push(TimelineEvent {
            event_type: EventType::Audio, start_ms, duration_ms,
            path: Some(path), text: Some(text), display_text: Some(display_text),
            cast: Some(cast),
        });
    }

    pub fn register_bgm(&mut self, path: PathBuf) {
        self.events_inner.push(TimelineEvent {
            event_type: EventType::BgmStart, start_ms: self.current_ms,
            duration_ms: 0.0, path: Some(path), text: None, display_text: None, cast: None,
        });
    }

    pub fn register_bgm_stop(&mut self) {
        self.events_inner.push(TimelineEvent {
            event_type: EventType::BgmStop, start_ms: self.current_ms,
            duration_ms: 0.0, path: None, text: None, display_text: None, cast: None,
        });
    }

    pub fn register_se(&mut self, path: PathBuf) {
        self.events_inner.push(TimelineEvent {
            event_type: EventType::Se, start_ms: self.current_ms,
            duration_ms: 0.0, path: Some(path), text: None, display_text: None, cast: None,
        });
    }

    pub fn advance_after_speech(&mut self, duration_ms: f64, pause_ms: f64) {
        self.current_ms += duration_ms + pause_ms;
    }

    pub fn advance_after_parallel(&mut self, anchor_ms: f64, max_occupied_ms: f64, pause_ms: f64) {
        self.current_ms = anchor_ms + max_occupied_ms + pause_ms;
    }

    pub fn advance_pause(&mut self, ms: f64) { self.current_ms += ms; }

    pub fn advance_paragraph(&mut self) { self.current_ms += self.paragraph_pause_ms; }

    pub fn sentence_pause_ms(&self) -> f64 { self.sentence_pause_ms }
    pub fn cast_pause_ms(&self) -> f64 { self.cast_pause_ms }

    pub fn events(&self) -> &[TimelineEvent] { &self.events_inner }
    pub fn into_events(self) -> Vec<TimelineEvent> { self.events_inner }
}
```

- [ ] **Step 3: lib.rs 更新**

```rust
pub mod timeline;
pub use timeline::{TimelineProcessor, TimelineEvent, EventType};
```

- [ ] **Step 4: テスト PASS 確認**

```
cargo test -p s2v-core
```
Expected: すべて PASS

- [ ] **Step 5: コミット**

```
git add crates/s2v-core/
git commit -m "feat(core): add TimelineProcessor and TimelineEvent"
```

---

## Task 7: s2v-engines — Engine trait + EngineManager

**Files:**
- Modify: `crates/s2v-engines/src/lib.rs`

- [ ] **Step 1: lib.rs 実装**

```rust
// crates/s2v-engines/src/lib.rs
pub mod http_engine;
pub mod xtts;

use std::{collections::{HashMap, HashSet}, path::Path, sync::Arc};
use async_trait::async_trait;
use anyhow::Result;
use s2v_core::{Cast, Config};

#[async_trait]
pub trait Engine: Send + Sync {
    async fn activate(&self) -> Result<()>;
    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> Result<()>;
    fn is_cast_valid(&self, cast: &Cast) -> bool { let _ = cast; true }
}

pub struct EngineManager {
    engines: HashMap<String, Arc<dyn Engine>>,
    semaphores: HashMap<String, Arc<tokio::sync::Semaphore>>,
}

impl EngineManager {
    pub fn from_config(config: &Config) -> Self {
        use http_engine::HttpEngine;
        use xtts::XttsEngine;
        let mut engines: HashMap<String, Arc<dyn Engine>> = HashMap::new();
        engines.insert("voicevox".into(), Arc::new(HttpEngine::new("voicevox", &config.voicevox.url)));
        engines.insert("aivis".into(),    Arc::new(HttpEngine::new("aivis",    &config.aivis.url)));
        engines.insert("xtts".into(),     Arc::new(XttsEngine::new(&config.xtts.url)));

        let mut sems = HashMap::new();
        sems.insert("voicevox".into(), Arc::new(tokio::sync::Semaphore::new(config.concurrency.voicevox)));
        sems.insert("aivis".into(),    Arc::new(tokio::sync::Semaphore::new(config.concurrency.aivis)));
        sems.insert("xtts".into(),     Arc::new(tokio::sync::Semaphore::new(config.concurrency.xtts)));

        Self { engines, semaphores: sems }
    }

    pub fn get(&self, engine_type: &str) -> Option<Arc<dyn Engine>> {
        self.engines.get(engine_type).cloned()
    }

    pub fn semaphore(&self, engine_type: &str) -> Option<Arc<tokio::sync::Semaphore>> {
        self.semaphores.get(engine_type).cloned()
    }

    pub async fn activate_required(&self, types: &HashSet<String>) -> Result<()> {
        let mut tasks = Vec::new();
        for t in types {
            if let Some(e) = self.get(t) {
                let t = t.clone();
                tasks.push(tokio::spawn(async move {
                    e.activate().await.map_err(|err| format!("[{t}] {err}"))
                }));
            }
        }
        for task in tasks {
            if let Err(e) = task.await? {
                tracing::warn!("Engine activation warning: {e}");
            }
        }
        Ok(())
    }

    pub async fn synthesize(&self, text: &str, cast: &Cast, out: &Path) -> Result<()> {
        let engine = self.get(&cast.engine_type)
            .ok_or_else(|| anyhow::anyhow!("Engine not found: {}", cast.engine_type))?;
        let sem = self.semaphore(&cast.engine_type);
        if let Some(sem) = sem {
            let _permit = sem.acquire().await?;
            engine.synthesize(text, cast, out).await
        } else {
            engine.synthesize(text, cast, out).await
        }
    }
}
```

- [ ] **Step 2: stub ファイルを作成**

```rust
// crates/s2v-engines/src/http_engine.rs
use async_trait::async_trait;
use anyhow::Result;
use s2v_core::Cast;
use std::path::Path;
use crate::Engine;

pub struct HttpEngine { name: String, url: String, client: reqwest::Client }

impl HttpEngine {
    pub fn new(name: &str, url: &str) -> Self {
        Self { name: name.to_string(), url: url.to_string(), client: reqwest::Client::new() }
    }
}

#[async_trait]
impl Engine for HttpEngine {
    async fn activate(&self) -> Result<()> { Ok(()) }  // Task 8 で実装
    async fn synthesize(&self, _text: &str, _cast: &Cast, _output: &Path) -> Result<()> { Ok(()) }
}
```

```rust
// crates/s2v-engines/src/xtts.rs
use async_trait::async_trait;
use anyhow::Result;
use s2v_core::Cast;
use std::path::Path;
use crate::Engine;

pub struct XttsEngine { url: String, client: reqwest::Client }

impl XttsEngine {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_string(), client: reqwest::Client::new() }
    }
}

#[async_trait]
impl Engine for XttsEngine {
    async fn activate(&self) -> Result<()> { Ok(()) }
    async fn synthesize(&self, _text: &str, _cast: &Cast, _output: &Path) -> Result<()> { Ok(()) }
}
```

- [ ] **Step 3: ビルド確認**

```
cargo build -p s2v-engines
```
Expected: PASS

- [ ] **Step 4: コミット**

```
git add crates/s2v-engines/
git commit -m "feat(engines): add Engine trait and EngineManager scaffold"
```

---

## Task 8: s2v-engines — HttpEngine (VOICEVOX / AivisSpeech)

**Files:**
- Modify: `crates/s2v-engines/src/http_engine.rs`

- [ ] **Step 1: テスト作成 (wiremock)**

```rust
// crates/s2v-engines/tests/engine_tests.rs
use s2v_engines::http_engine::HttpEngine;
use s2v_engines::Engine;
use s2v_core::Cast;
use std::collections::HashMap;
use wiremock::{MockServer, Mock, ResponseTemplate};
use wiremock::matchers::{method, path};

fn make_cast(engine: &str) -> Cast {
    let mut params = HashMap::new();
    params.insert("style".to_string(), serde_json::Value::String("ノーマル".into()));
    Cast { name: "A".into(), speaker_name: "四国めたん".into(),
           engine_type: engine.into(), pan: 0.0, distance: 1.0, volume: 1.0, params }
}

#[tokio::test]
async fn test_voicevox_synthesize() {
    let server = MockServer::start().await;
    // /speakers
    Mock::given(method("GET")).and(path("/speakers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"name":"四国めたん","styles":[{"name":"ノーマル","id":2}]}
        ])))
        .mount(&server).await;
    // /audio_query
    Mock::given(method("POST")).and(path("/audio_query"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"speedScale":1.0})))
        .mount(&server).await;
    // /synthesis → 最小WAVバイト列
    let wav_bytes = make_minimal_wav();
    Mock::given(method("POST")).and(path("/synthesis"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(wav_bytes))
        .mount(&server).await;

    let engine = HttpEngine::new("voicevox", &server.uri());
    engine.activate().await.unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    engine.synthesize("テスト", &make_cast("voicevox"), tmp.path()).await.unwrap();
    assert!(tmp.path().exists());
}

fn make_minimal_wav() -> Vec<u8> {
    // 44-byte WAV header + 2 bytes (1 sample, 16-bit mono, 24000Hz)
    let mut w = Vec::new();
    let data_size: u32 = 2;
    let total_size: u32 = 36 + data_size;
    w.extend_from_slice(b"RIFF"); w.extend_from_slice(&total_size.to_le_bytes());
    w.extend_from_slice(b"WAVEfmt "); w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&24000u32.to_le_bytes()); w.extend_from_slice(&48000u32.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes()); w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data"); w.extend_from_slice(&data_size.to_le_bytes());
    w.extend_from_slice(&0i16.to_le_bytes()); w
}
```

- [ ] **Step 2: テスト失敗確認**

```
cargo test -p s2v-engines --test engine_tests test_voicevox_synthesize 2>&1 | head -5
```
Expected: FAIL

- [ ] **Step 3: http_engine.rs 完全実装**

```rust
// crates/s2v-engines/src/http_engine.rs
use async_trait::async_trait;
use anyhow::{Context, Result};
use s2v_core::Cast;
use serde_json::Value;
use std::{collections::HashMap, path::Path, sync::Arc};
use tokio::sync::RwLock;
use crate::Engine;

type SpeakerCache = HashMap<String, HashMap<String, u32>>; // 話者名 → {スタイル名 → ID}

pub struct HttpEngine {
    name: String,
    url: String,
    client: reqwest::Client,
    cache: Arc<RwLock<SpeakerCache>>,
}

impl HttpEngine {
    pub fn new(name: &str, url: &str) -> Self {
        Self { name: name.to_string(), url: url.to_string(),
               client: reqwest::Client::builder().timeout(std::time::Duration::from_secs(60)).build().unwrap(),
               cache: Arc::new(RwLock::new(HashMap::new())) }
    }

    async fn refresh_cache(&self) -> Result<()> {
        let resp: Value = self.client.get(format!("{}/speakers", self.url))
            .send().await?.json().await?;
        let mut cache = self.cache.write().await;
        *cache = resp.as_array().unwrap_or(&vec![]).iter().filter_map(|s| {
            let name = s["name"].as_str()?.to_string();
            let styles: HashMap<String, u32> = s["styles"].as_array()?.iter().filter_map(|st| {
                Some((st["name"].as_str()?.to_string(), st["id"].as_u64()? as u32))
            }).collect();
            Some((name, styles))
        }).collect();
        Ok(())
    }
}

#[async_trait]
impl Engine for HttpEngine {
    async fn activate(&self) -> Result<()> {
        self.client.get(format!("{}/version", self.url))
            .timeout(std::time::Duration::from_secs(3))
            .send().await.context("engine not reachable")?;
        self.refresh_cache().await?;
        tracing::info!("[{}] activated, {} speakers", self.name, self.cache.read().await.len());
        Ok(())
    }

    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> Result<()> {
        let cache = self.cache.read().await;
        let styles = cache.get(&cast.speaker_name)
            .ok_or_else(|| anyhow::anyhow!("[{}] speaker '{}' not found", self.name, cast.speaker_name))?;
        let style_name = cast.params.get("style").and_then(|v| v.as_str()).unwrap_or("ノーマル");
        let style_id = styles.get(style_name)
            .or_else(|| styles.values().next())
            .copied()
            .ok_or_else(|| anyhow::anyhow!("[{}] no styles for '{}'", self.name, cast.speaker_name))?;
        drop(cache);

        // audio_query
        let mut query: Value = self.client.post(format!("{}/audio_query", self.url))
            .query(&[("text", text), ("speaker", &style_id.to_string())])
            .send().await?.json().await?;

        // キャストパラメータをクエリに上書き
        if let Value::Object(ref mut map) = query {
            for (k, v) in &cast.params {
                if k != "style" && map.contains_key(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
        }

        // synthesis
        let bytes = self.client.post(format!("{}/synthesis", self.url))
            .query(&[("speaker", style_id.to_string())])
            .json(&query)
            .timeout(std::time::Duration::from_secs(120))
            .send().await?.bytes().await?;

        std::fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
        std::fs::write(output, &bytes)?;
        Ok(())
    }

    fn is_cast_valid(&self, cast: &Cast) -> bool {
        // キャッシュが空なら検証スキップ（activate前）
        let cache = match self.cache.try_read() { Ok(c) => c, Err(_) => return true };
        cache.contains_key(&cast.speaker_name)
    }
}
```

- [ ] **Step 4: dev-dependencies に tempfile 追加、テスト PASS 確認**

`crates/s2v-engines/Cargo.toml` の `[dev-dependencies]` に `tempfile = "3"` を追加。

```
cargo test -p s2v-engines --test engine_tests
```
Expected: PASS

- [ ] **Step 5: コミット**

```
git add crates/s2v-engines/
git commit -m "feat(engines): implement HttpEngine for VOICEVOX/AivisSpeech"
```

---

## Task 9: s2v-engines — XttsEngine

**Files:**
- Modify: `crates/s2v-engines/src/xtts.rs`
- Modify: `crates/s2v-engines/tests/engine_tests.rs`

- [ ] **Step 1: テスト追加**

```rust
// crates/s2v-engines/tests/engine_tests.rs に追加
use s2v_engines::xtts::XttsEngine;

#[tokio::test]
async fn test_xtts_synthesize() {
    let server = MockServer::start().await;
    Mock::given(method("GET")).and(path("/speakers"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
            [{"name":"narrator"}]
        ))).mount(&server).await;
    Mock::given(method("POST")).and(path("/get_tts_settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"speed":1.0})))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/set_tts_settings"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server).await;
    Mock::given(method("POST")).and(path("/tts_to_audio/"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(make_minimal_wav()))
        .mount(&server).await;

    let engine = XttsEngine::new(&server.uri());
    engine.activate().await.unwrap();
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let cast = make_cast("xtts");
    engine.synthesize("hello", &cast, tmp.path()).await.unwrap();
    assert!(tmp.path().exists());
}
```

- [ ] **Step 2: xtts.rs 完全実装**

```rust
// crates/s2v-engines/src/xtts.rs
use async_trait::async_trait;
use anyhow::Result;
use s2v_core::Cast;
use serde_json::Value;
use std::{collections::HashSet, path::Path, sync::Arc};
use tokio::sync::RwLock;
use crate::Engine;

pub struct XttsEngine {
    url: String,
    client: reqwest::Client,
    speakers: Arc<RwLock<HashSet<String>>>,
}

impl XttsEngine {
    pub fn new(url: &str) -> Self {
        Self { url: url.to_string(),
               client: reqwest::Client::builder().timeout(std::time::Duration::from_secs(120)).build().unwrap(),
               speakers: Arc::new(RwLock::new(HashSet::new())) }
    }
}

#[async_trait]
impl Engine for XttsEngine {
    async fn activate(&self) -> Result<()> {
        let resp: Value = self.client.get(format!("{}/speakers", self.url))
            .timeout(std::time::Duration::from_secs(60))
            .send().await?.json().await?;
        let mut set = self.speakers.write().await;
        *set = resp.as_array().unwrap_or(&vec![]).iter()
            .filter_map(|s| s["name"].as_str().map(|n| n.to_string()))
            .collect();
        tracing::info!("[xtts] activated, {} speakers", set.len());
        Ok(())
    }

    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> Result<()> {
        // 設定取得 → パラメータ上書き → 設定送信
        let mut settings: Value = self.client.post(format!("{}/get_tts_settings", self.url))
            .send().await?.json().await?;
        if let Value::Object(ref mut map) = settings {
            for (k, v) in &cast.params {
                if k != "style" && k != "language" && map.contains_key(k) {
                    map.insert(k.clone(), v.clone());
                }
            }
        }
        let _ = self.client.post(format!("{}/set_tts_settings", self.url))
            .json(&settings).send().await?;

        let language = cast.params.get("language").and_then(|v| v.as_str()).unwrap_or("ja");
        let payload = serde_json::json!({
            "text": text,
            "speaker_name": cast.speaker_name,
            "language": language,
        });
        let bytes = self.client.post(format!("{}/tts_to_audio/", self.url))
            .json(&payload).send().await?.bytes().await?;

        std::fs::create_dir_all(output.parent().unwrap_or(Path::new(".")))?;
        std::fs::write(output, &bytes)?;
        Ok(())
    }
}
```

- [ ] **Step 3: テスト PASS**

```
cargo test -p s2v-engines
```
Expected: PASS

- [ ] **Step 4: コミット**

```
git add crates/s2v-engines/
git commit -m "feat(engines): implement XttsEngine"
```

---

## Task 10: s2v-audio — Resampler

**Files:**
- Create: `crates/s2v-audio/src/resample.rs`
- Modify: `crates/s2v-audio/src/lib.rs`

- [ ] **Step 1: テスト作成**

```rust
// crates/s2v-audio/tests/audio_tests.rs
use s2v_audio::resample::resample_mono;

#[test]
fn test_resample_24k_to_48k() {
    // 1秒の 440Hz サイン波 (24kHz)
    let src_rate = 24000usize;
    let dst_rate = 48000usize;
    let samples: Vec<f32> = (0..src_rate)
        .map(|i| (2.0 * std::f32::consts::PI * 440.0 * i as f32 / src_rate as f32).sin())
        .collect();
    let out = resample_mono(&samples, src_rate, dst_rate).unwrap();
    // 出力サンプル数が dst_rate に近い（±1%）
    let ratio = out.len() as f64 / dst_rate as f64;
    assert!((ratio - 1.0).abs() < 0.01, "ratio={ratio}");
}

#[test]
fn test_resample_noop() {
    let samples: Vec<f32> = vec![0.1, 0.2, 0.3, 0.4];
    let out = resample_mono(&samples, 48000, 48000).unwrap();
    assert_eq!(out.len(), samples.len());
}
```

- [ ] **Step 2: resample.rs 実装**

```rust
// crates/s2v-audio/src/resample.rs
use anyhow::Result;
use rubato::{FftFixedIn, Resampler};

/// モノラル f32 サンプル列を src_rate から dst_rate にリサンプリング
pub fn resample_mono(input: &[f32], src_rate: usize, dst_rate: usize) -> Result<Vec<f32>> {
    if src_rate == dst_rate { return Ok(input.to_vec()); }
    if input.is_empty() { return Ok(Vec::new()); }

    let chunk = input.len().min(8192);
    let mut resampler = FftFixedIn::<f32>::new(src_rate, dst_rate, chunk, 2, 1)?;
    let mut out = Vec::new();
    let mut pos = 0;

    while pos < input.len() {
        let end = (pos + chunk).min(input.len());
        let mut chunk_data = input[pos..end].to_vec();
        chunk_data.resize(chunk, 0.0);  // 末尾ゼロパディング
        let result = resampler.process(&[chunk_data], None)?;
        out.extend_from_slice(&result[0]);
        pos = end;
        if end == input.len() { break; }
    }
    // 厳密なサンプル数に切り詰め
    let expected = (input.len() as f64 * dst_rate as f64 / src_rate as f64).round() as usize;
    out.truncate(expected.max(1));
    Ok(out)
}
```

- [ ] **Step 3: lib.rs 更新**

```rust
// crates/s2v-audio/src/lib.rs
pub mod resample;
pub mod reverb;
pub mod processor;
```

`reverb.rs` と `processor.rs` は空ファイルで OK（次のタスクで実装）。

- [ ] **Step 4: テスト PASS**

```
cargo test -p s2v-audio --test audio_tests test_resample
```
Expected: PASS

- [ ] **Step 5: コミット**

```
git add crates/s2v-audio/
git commit -m "feat(audio): add resample_mono with rubato FftFixedIn"
```

---

## Task 11: s2v-audio — Reverb (IR生成 + realfft畳み込み)

**Files:**
- Modify: `crates/s2v-audio/src/reverb.rs`

- [ ] **Step 1: テスト追加**

```rust
// crates/s2v-audio/tests/audio_tests.rs に追加
use s2v_audio::reverb::ReverbEngine;

#[test]
fn test_ir_cache() {
    let engine = ReverbEngine::new(48000);
    engine.prewarm(&[0.3, 0.5]);
    // 同じ room_size で IR を 2 回取得しても同じ長さ
    let ir1 = engine.get_ir(0.3);
    let ir2 = engine.get_ir(0.3);
    assert_eq!(ir1[0].len(), ir2[0].len());
    assert!(ir1[0].len() > 0);
}

#[test]
fn test_apply_reverb_no_panic() {
    let engine = ReverbEngine::new(48000);
    let stereo: Vec<[f32; 2]> = vec![[0.5, -0.5]; 480]; // 10ms
    let result = engine.apply(&stereo, 1.0, 0.3, 0.5);
    assert_eq!(result.len(), stereo.len());
}
```

- [ ] **Step 2: reverb.rs 実装**

```rust
// crates/s2v-audio/src/reverb.rs
use std::{collections::HashMap, sync::Mutex};
use ordered_float::OrderedFloat;
use rand::{SeedableRng, Rng};
use rand::rngs::SmallRng;
use rand_distr::{Distribution, Normal};
use realfft::RealFftPlanner;

pub struct ReverbEngine {
    fs: u32,
    ir_cache: Mutex<HashMap<OrderedFloat<f64>, [[Vec<f32>; 2]; 1]>>,
    lp_coeffs: ([f64; 3], [f64; 3]),  // (b, a) 2次 Butterworth LP @1800Hz
}

impl ReverbEngine {
    pub fn new(fs: u32) -> Self {
        Self { fs, ir_cache: Mutex::new(HashMap::new()), lp_coeffs: butterworth2_lp(1800.0, fs as f64) }
    }

    pub fn prewarm(&self, room_sizes: &[f64]) {
        for &rs in room_sizes { self.compute_ir_if_needed(rs); }
    }

    pub fn get_ir(&self, room_size: f64) -> [Vec<f32>; 2] {
        self.compute_ir_if_needed(room_size);
        let cache = self.ir_cache.lock().unwrap();
        let entry = &cache[&OrderedFloat(round4(room_size))];
        [entry[0][0].clone(), entry[0][1].clone()]
    }

    fn compute_ir_if_needed(&self, room_size: f64) {
        let key = OrderedFloat(round4(room_size));
        let mut cache = self.ir_cache.lock().unwrap();
        if cache.contains_key(&key) { return; }

        let fs = self.fs as f64;
        let rv_time = 0.05 + room_size * 3.0;
        let pre_delay = (fs * (0.01 + 0.04 * room_size)) as usize;
        let n = (fs * rv_time) as usize;
        let seed = (room_size * 10000.0) as u64 & 0xFFFFFFFF;
        let mut rng = SmallRng::seed_from_u64(seed);
        let normal = Normal::new(0.0f64, 1.0).unwrap();
        let decay: Vec<f64> = (0..n).map(|i| (-6.91 * i as f64 / (fs * rv_time)).exp()).collect();

        let mut irs = [Vec::new(), Vec::new()];
        for ch in 0..2 {
            let noise: Vec<f32> = (0..n).map(|_| normal.sample(&mut rng) as f32).collect();
            let filtered = apply_iir_f32(&noise, self.lp_coeffs.0, self.lp_coeffs.1);
            let mut ir = vec![0.0f32; pre_delay];
            for (i, &s) in filtered.iter().enumerate() {
                ir.push(s * decay[i] as f32);
            }
            irs[ch] = ir;
        }
        cache.insert(key, [[irs[0].clone(), irs[1].clone()]]);
    }

    /// ステレオバッファにリバーブを適用して返す（元の長さに切り詰め）
    pub fn apply(&self, stereo: &[[f32; 2]], avg_dist: f64, room_size: f64, reverb_wet: f64) -> Vec<[f32; 2]> {
        if reverb_wet <= 0.0 { return stereo.to_vec(); }
        let actual_wet = (reverb_wet * (1.0 + 0.1 * avg_dist)).min(0.9) as f32;
        let irs = self.get_ir(room_size);
        let out_len = stereo.len();
        let mut result = stereo.to_vec();

        for ch in 0..2 {
            let dry: Vec<f32> = stereo.iter().map(|s| s[ch]).collect();
            let wet = fft_convolve(&dry, &irs[ch]);
            let dry_peak = dry.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
            let wet_peak = wet.iter().map(|s| s.abs()).fold(f32::EPSILON, f32::max);
            let wet_norm = wet_peak.max(1e-6);
            for i in 0..out_len {
                let w = if i < wet.len() { wet[i] / wet_norm * dry_peak * 0.4 } else { 0.0 };
                result[i][ch] = (1.0 - actual_wet) * result[i][ch] + actual_wet * w;
            }
        }
        result
    }
}

fn fft_convolve(signal: &[f32], kernel: &[f32]) -> Vec<f32> {
    let out_len = signal.len() + kernel.len() - 1;
    let fft_len = next_power_of_two(out_len);
    let mut planner = RealFftPlanner::<f32>::new();
    let fft  = planner.plan_fft_forward(fft_len);
    let ifft = planner.plan_fft_inverse(fft_len);

    let mut s = signal.to_vec(); s.resize(fft_len, 0.0);
    let mut k = kernel.to_vec();  k.resize(fft_len, 0.0);
    let mut S = fft.make_output_vec();
    let mut K = fft.make_output_vec();
    fft.process(&mut s, &mut S).unwrap();
    fft.process(&mut k, &mut K).unwrap();

    let mut product: Vec<_> = S.iter().zip(K.iter()).map(|(a, b)| a * b).collect();
    let mut out = ifft.make_output_vec();
    ifft.process(&mut product, &mut out).unwrap();
    let scale = 1.0 / fft_len as f32;
    out.iter().take(out_len).map(|s| s * scale).collect()
}

fn next_power_of_two(n: usize) -> usize {
    let mut p = 1; while p < n { p <<= 1; } p
}

fn round4(v: f64) -> f64 { (v * 10000.0).round() / 10000.0 }

fn butterworth2_lp(fc: f64, fs: f64) -> ([f64; 3], [f64; 3]) {
    let k = (std::f64::consts::PI * fc / fs).tan();
    let k2 = k * k; let sqrt2 = std::f64::consts::SQRT_2;
    let d = k2 + sqrt2 * k + 1.0;
    ([k2/d, 2.0*k2/d, k2/d], [1.0, 2.0*(k2-1.0)/d, (k2 - sqrt2*k + 1.0)/d])
}

fn apply_iir_f32(data: &[f32], b: [f64; 3], a: [f64; 3]) -> Vec<f32> {
    let mut out = vec![0.0f32; data.len()];
    let mut z = [0.0f64; 2];
    for (i, &x) in data.iter().enumerate() {
        let x = x as f64;
        let y = b[0]*x + z[0];
        z[0] = b[1]*x - a[1]*y + z[1];
        z[1] = b[2]*x - a[2]*y;
        out[i] = y as f32;
    }
    out
}
```

- [ ] **Step 3: テスト PASS**

```
cargo test -p s2v-audio --test audio_tests test_ir test_apply_reverb
```
Expected: PASS

- [ ] **Step 4: コミット**

```
git add crates/s2v-audio/
git commit -m "feat(audio): implement ReverbEngine with IR cache and FFT convolution"
```

---

## Task 12: s2v-audio — AudioProcessor

**Files:**
- Modify: `crates/s2v-audio/src/processor.rs`

- [ ] **Step 1: テスト追加**

```rust
// crates/s2v-audio/tests/audio_tests.rs に追加
use s2v_core::{Cast, Config};
use s2v_audio::processor::AudioProcessor;
use std::collections::HashMap;

fn make_config() -> s2v_core::config::AudioConfig {
    s2v_core::config::AudioConfig {
        sample_rate: 48000, microphone_spacing: 0.2, sound_speed: 340.0,
        air_absorption_coeff: 0.05, room_size: 0.1, reverb_wet: 0.3,
        reference_dist: 1.0, reference_gain_db: -5.0, max_gain_db: -1.0,
        mic_directivity: 0.5, mic_angle: 45.0,
        engine_volume_offsets: [("voicevox".to_string(), 1.0)].into_iter().collect(),
    }
}

#[test]
fn test_process_returns_stereo() {
    let proc = AudioProcessor::new(make_config());
    let input: Vec<f32> = (0..4800).map(|i| (i as f32 * 0.01).sin() * 0.1).collect();
    let scene = s2v_core::parser::SceneConfig { room_size: None, reverb_wet: None };
    let cast = Cast { name: "A".into(), speaker_name: "sp".into(), engine_type: "voicevox".into(),
                      pan: 0.0, distance: 1.0, volume: 1.0, params: HashMap::new() };
    let result = proc.process(&input, 48000, &cast, &scene);
    assert!(result.len() >= input.len()); // リバーブ尾部で伸びる場合あり
    // ステレオ確認: 両チャンネルが非ゼロ
    let has_signal = result.iter().any(|s| s[0].abs() > 1e-6 || s[1].abs() > 1e-6);
    assert!(has_signal);
}
```

- [ ] **Step 2: processor.rs 実装**

```rust
// crates/s2v-audio/src/processor.rs
use std::collections::HashMap;
use s2v_core::{Cast, config::AudioConfig, parser::SceneConfig};
use crate::{resample::resample_mono, reverb::ReverbEngine};

pub struct AudioProcessor {
    cfg: AudioConfig,
    reverb: ReverbEngine,
    max_gain: f32,
}

impl AudioProcessor {
    pub fn new(cfg: AudioConfig) -> Self {
        let max_gain = 10.0f32.powf(cfg.max_gain_db as f32 / 20.0);
        let reverb = ReverbEngine::new(cfg.sample_rate);
        Self { cfg, reverb, max_gain }
    }

    pub fn prewarm_ir_cache(&self, room_sizes: &[f64]) {
        self.reverb.prewarm(room_sizes);
    }

    /// 音響処理のフルパイプライン。入力サンプル数の周波数は src_rate で指定。
    pub fn process(&self, input: &[f32], src_rate: u32, cast: &Cast, scene: &SceneConfig) -> Vec<[f32; 2]> {
        let fs = self.cfg.sample_rate;

        // 1. リサンプリング
        let mono = if src_rate != fs {
            resample_mono(input, src_rate as usize, fs as usize).unwrap_or_else(|_| input.to_vec())
        } else { input.to_vec() };

        // 2. 正規化
        let peak = mono.iter().map(|s| s.abs()).fold(0.0f32, f32::max);
        let mono: Vec<f32> = if peak > 0.0 { mono.iter().map(|s| s / peak).collect() } else { mono };

        // 3. 幾何学計算
        let pan_rad = cast.pan.to_radians() as f64;
        let geo = self.calc_geometry(cast.distance, pan_rad);

        // 4. 空気吸収（チャンネル別）
        let data_l = self.air_absorption(&mono, geo.dist_l);
        let data_r = self.air_absorption(&mono, geo.dist_r);

        // 5. ITD (サンプル遅延)
        let v = self.cfg.sound_speed;
        let delay_l = ((geo.dist_l / v) * fs as f64) as usize;
        let delay_r = ((geo.dist_r / v) * fs as f64) as usize;
        let min_delay = delay_l.min(delay_r);
        let rel_l = delay_l - min_delay;
        let rel_r = delay_r - min_delay;

        // 6. ILD + 音量
        let ref_db = 10.0f64.powf(self.cfg.reference_gain_db / 20.0);
        let nom_pat = (1.0 - self.cfg.mic_directivity) + self.cfg.mic_directivity * (self.cfg.mic_angle.to_radians()).cos();
        let base_norm = self.max_gain as f64 / nom_pat.max(1e-6) * ref_db / self.cfg.reference_dist;
        let engine_vol = self.cfg.engine_volume_offsets.get(&cast.engine_type).copied().unwrap_or(1.0);
        let vol_factor = base_norm * cast.volume * engine_vol;

        let mic_angle = self.cfg.mic_angle.to_radians();
        let pat_l = ((1.0-self.cfg.mic_directivity) + self.cfg.mic_directivity * (geo.angle_l - mic_angle).cos()).max(0.01);
        let pat_r = ((1.0-self.cfg.mic_directivity) + self.cfg.mic_directivity * (geo.angle_r + mic_angle).cos()).max(0.01);
        let gain_l = (vol_factor * self.cfg.reference_dist / geo.dist_l.max(0.1) * pat_l) as f32;
        let gain_r = (vol_factor * self.cfg.reference_dist / geo.dist_r.max(0.1) * pat_r) as f32;

        // 7. ステレオバッファ構築
        let rv_time = 0.05 + self.eff_room_size(cast, scene) * 3.0;
        let rv_samples = (fs as f64 * rv_time) as usize;
        let out_len = mono.len() + rel_l.max(rel_r) + rv_samples;
        let mut stereo = vec![[0.0f32; 2]; out_len];
        for (i, (&l, &r)) in data_l.iter().zip(data_r.iter()).enumerate() {
            if rel_l + i < out_len { stereo[rel_l + i][0] = l * gain_l; }
            if rel_r + i < out_len { stereo[rel_r + i][1] = r * gain_r; }
        }

        // 8. リバーブ
        let room = self.eff_room_size(cast, scene);
        let wet  = self.eff_reverb_wet(cast, scene);
        let stereo = self.reverb.apply(&stereo, cast.distance, room, wet);

        // 9. リミッター
        let peak = stereo.iter().flat_map(|s| s.iter()).map(|s| s.abs()).fold(0.0f32, f32::max);
        if peak > self.max_gain { stereo.iter().map(|s| [s[0]*self.max_gain/peak, s[1]*self.max_gain/peak]).collect() }
        else { stereo }
    }

    fn eff_room_size(&self, cast: &Cast, scene: &SceneConfig) -> f64 {
        cast.params.get("room_size").and_then(|v| v.as_f64())
            .or(scene.room_size)
            .unwrap_or(self.cfg.room_size)
    }
    fn eff_reverb_wet(&self, cast: &Cast, scene: &SceneConfig) -> f64 {
        cast.params.get("reverb_wet").and_then(|v| v.as_f64())
            .or(scene.reverb_wet)
            .unwrap_or(self.cfg.reverb_wet)
    }

    struct Geo { dist_l: f64, dist_r: f64, angle_l: f64, angle_r: f64 }

    fn calc_geometry(&self, r: f64, pan_rad: f64) -> Geo {
        let d_h = self.cfg.microphone_spacing / 2.0;
        let sx = r * pan_rad.sin();
        let sy = r * pan_rad.cos();
        let dist_l  = ((sx + d_h).powi(2) + sy.powi(2)).sqrt();
        let dist_r  = ((sx - d_h).powi(2) + sy.powi(2)).sqrt();
        let angle_l = (sx + d_h).atan2(sy);
        let angle_r = (sx - d_h).atan2(sy);
        Geo { dist_l, dist_r, angle_l, angle_r }
    }

    fn air_absorption(&self, data: &[f32], dist: f64) -> Vec<f32> {
        let coeff = self.cfg.air_absorption_coeff;
        if coeff <= 0.0 { return data.to_vec(); }
        let fs = self.cfg.sample_rate as f64;
        let cutoff = (fs / 2.0 / (1.0 + coeff * dist)).min(fs / 2.0 - 100.0);
        let k = (std::f64::consts::PI * cutoff / fs).tan();
        let d = 1.0 + k;
        let b0 = k / d; let a1 = (k - 1.0) / d;
        let mut out = vec![0.0f32; data.len()];
        let mut y_prev = 0.0f64; let mut x_prev = 0.0f64;
        for (i, &x) in data.iter().enumerate() {
            let x = x as f64;
            let y = b0 * (x + x_prev) - a1 * y_prev;
            x_prev = x; y_prev = y;
            out[i] = y as f32;
        }
        out
    }
}
```

- [ ] **Step 3: テスト PASS**

```
cargo test -p s2v-audio
```
Expected: PASS

- [ ] **Step 4: コミット**

```
git add crates/s2v-audio/
git commit -m "feat(audio): implement AudioProcessor with full DSP pipeline"
```

---

## Task 13: s2v-export — SRT / FCPXML / WAV ミックス

**Files:**
- Modify: `crates/s2v-export/src/srt.rs`
- Modify: `crates/s2v-export/src/fcpxml.rs`
- Modify: `crates/s2v-export/src/mix.rs`
- Modify: `crates/s2v-export/src/lib.rs`

- [ ] **Step 1: テスト作成**

```rust
// crates/s2v-export/tests/export_tests.rs
use s2v_export::srt::format_srt_time;

#[test]
fn test_srt_time_format() {
    assert_eq!(format_srt_time(0.0),    "00:00:00,000");
    assert_eq!(format_srt_time(1.5),    "00:00:01,500");
    assert_eq!(format_srt_time(3661.1), "01:01:01,100");
}
```

- [ ] **Step 2: srt.rs 実装**

```rust
// crates/s2v-export/src/srt.rs
use std::{fs, io::Write, path::Path};
use s2v_core::timeline::{TimelineEvent, EventType};
use anyhow::Result;

pub fn format_srt_time(secs: f64) -> String {
    let total_ms = (secs * 1000.0).round() as u64;
    let ms = total_ms % 1000;
    let s  = (total_ms / 1000) % 60;
    let m  = (total_ms / 60000) % 60;
    let h  = total_ms / 3600000;
    format!("{h:02}:{m:02}:{s:02},{ms:03}")
}

pub fn generate_srt(events: &[TimelineEvent], output_dir: &Path) -> Result<()> {
    let path = output_dir.join("timeline").join("subtitles.srt");
    fs::create_dir_all(path.parent().unwrap())?;
    let mut f = fs::File::create(&path)?;
    let mut idx = 1;
    for e in events.iter().filter(|e| e.event_type == EventType::Audio) {
        let start = format_srt_time(e.start_ms / 1000.0);
        let end   = format_srt_time((e.start_ms + e.duration_ms) / 1000.0);
        let text  = e.display_text.as_deref().unwrap_or("");
        writeln!(f, "{idx}\n{start} --> {end}\n{text}\n")?;
        idx += 1;
    }
    tracing::info!("SRT: {}", path.display());
    Ok(())
}
```

- [ ] **Step 3: fcpxml.rs 実装**

```rust
// crates/s2v-export/src/fcpxml.rs
use std::{fs, path::Path};
use s2v_core::timeline::{TimelineEvent, EventType};
use anyhow::Result;

pub fn generate_fcpxml(events: &[TimelineEvent], output_dir: &Path, crossfade_s: f64) -> Result<()> {
    let path = output_dir.join("timeline").join("timeline.fcpxml");
    fs::create_dir_all(path.parent().unwrap())?;

    let total_s = events.iter().map(|e| (e.start_ms + e.duration_ms) / 1000.0).fold(0.0f64, f64::max);
    let total_ticks = (total_s * 30000.0) as i64;

    let resources = events.iter().enumerate()
        .filter(|(_, e)| e.path.is_some())
        .map(|(i, e)| {
            let p = e.path.as_ref().unwrap();
            let url = format!("file://{}", p.to_string_lossy().replace('\\', "/"));
            format!(r#"        <asset id="a{i}" name="{}" src="{url}"/>"#,
                p.file_name().and_then(|n| n.to_str()).unwrap_or(""))
        }).collect::<Vec<_>>().join("\n");

    let clips = events.iter().enumerate()
        .filter_map(|(i, e)| {
            let start = (e.start_ms / 1000.0 * 30000.0) as i64;
            let dur   = (e.duration_ms / 1000.0 * 30000.0) as i64;
            if dur == 0 { return None; }
            let role = match e.event_type {
                EventType::Audio    => "dialogue",
                EventType::BgmStart => "music",
                EventType::Se       => "effects",
                EventType::BgmStop  => return None,
            };
            Some(format!(r#"                            <audio ref="a{i}" lane="{i}" offset="{start}/30000s" duration="{dur}/30000s" role="{role}"/>"#))
        }).collect::<Vec<_>>().join("\n");

    let xml = format!(r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE fcpxml>
<fcpxml version="1.8">
    <resources>
        <format id="r1" name="FFVideoFormat1080p2997" frameDuration="1001/30000s"/>
{resources}
    </resources>
    <library>
        <event name="VoiceProduction">
            <project name="ScriptTimeline">
                <sequence format="r1" duration="{total_ticks}/30000s">
                    <spine>
                        <gap name="Gap" offset="0s" duration="{total_ticks}/30000s">
{clips}
                        </gap>
                    </spine>
                </sequence>
            </project>
        </event>
    </library>
</fcpxml>"#);
    fs::write(&path, xml)?;
    tracing::info!("FCPXML: {}", path.display());
    Ok(())
}
```

- [ ] **Step 4: mix.rs 実装**

```rust
// crates/s2v-export/src/mix.rs
use std::path::Path;
use s2v_core::timeline::{TimelineEvent, EventType};
use anyhow::Result;

pub fn generate_combined_audio(
    events: &[TimelineEvent],
    output_dir: &Path,
    sample_rate: u32,
    se_fade_out_s: f64,
    bgm_crossfade_s: f64,
) -> Result<()> {
    let out_path = output_dir.join("full_dialogue.wav");
    let audio_events: Vec<_> = events.iter().filter(|e| e.event_type == EventType::Audio && e.path.is_some()).collect();
    if audio_events.is_empty() {
        tracing::warn!("No audio events, skipping mix");
        return Ok(());
    }

    // 総サンプル数を計算
    let mut total_samples = 0usize;
    let clips: Vec<_> = audio_events.iter().filter_map(|e| {
        let p = e.path.as_ref()?;
        let data = read_wav_stereo(p, sample_rate).ok()?;
        let start = (e.start_ms / 1000.0 * sample_rate as f64) as usize;
        total_samples = total_samples.max(start + data.len());
        Some((start, data))
    }).collect();

    if total_samples == 0 { return Ok(()); }

    let mut out = vec![[0.0f32; 2]; total_samples];
    for (start, data) in &clips {
        for (i, &s) in data.iter().enumerate() {
            let idx = start + i;
            if idx < out.len() { out[idx][0] += s[0]; out[idx][1] += s[1]; }
        }
    }

    // BGM ミックス
    mix_bgm(events, &mut out, sample_rate, bgm_crossfade_s);
    // SE ミックス
    mix_se(events, &mut out, sample_rate, se_fade_out_s);

    // クリッピング防止
    let peak = out.iter().flat_map(|s| s.iter()).map(|s| s.abs()).fold(0.0f32, f32::max);
    if peak > 1.0 { for s in &mut out { s[0] /= peak; s[1] /= peak; } }

    // WAV 書き出し (hound)
    let spec = hound::WavSpec { channels: 2, sample_rate, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut writer = hound::WavWriter::create(&out_path, spec)?;
    for s in &out {
        writer.write_sample((s[0] * 32767.0) as i16)?;
        writer.write_sample((s[1] * 32767.0) as i16)?;
    }
    writer.finalize()?;
    tracing::info!("Mixed WAV: {}", out_path.display());
    Ok(())
}

fn read_wav_stereo(path: &Path, target_fs: u32) -> Result<Vec<[f32; 2]>> {
    let mut reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let raw: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Int => reader.samples::<i16>()
            .map(|s| s.map(|v| v as f32 / 32768.0))
            .collect::<Result<_, _>>()?,
        hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_, _>>()?,
    };
    let mono: Vec<f32> = if spec.channels == 2 {
        raw.chunks(2).map(|c| (c[0] + c[1]) / 2.0).collect()
    } else { raw };

    // リサンプリング
    let mono = if spec.sample_rate != target_fs {
        s2v_audio::resample::resample_mono(&mono, spec.sample_rate as usize, target_fs as usize)?
    } else { mono };

    Ok(mono.into_iter().map(|s| [s, s]).collect())
}

fn mix_bgm(events: &[TimelineEvent], out: &mut [[f32; 2]], fs: u32, crossfade_s: f64) {
    // BGM セグメントを収集してミックス（クロスフェード付き）
    let mut pending: Option<(usize, f64, &Path)> = None;
    let total_s = out.len() as f64 / fs as f64;

    for e in events {
        match e.event_type {
            EventType::BgmStart => {
                if let Some((pi, ps, pp)) = pending {
                    mix_bgm_segment(out, fs, pp, ps, e.start_ms / 1000.0, crossfade_s);
                }
                pending = e.path.as_deref().map(|p| (0, e.start_ms / 1000.0, p));
            }
            EventType::BgmStop => {
                if let Some((_, ps, pp)) = pending {
                    mix_bgm_segment(out, fs, pp, ps, e.start_ms / 1000.0, crossfade_s);
                    pending = None;
                }
            }
            _ => {}
        }
    }
    if let Some((_, ps, pp)) = pending {
        mix_bgm_segment(out, fs, pp, ps, total_s, crossfade_s);
    }
}

fn mix_bgm_segment(out: &mut [[f32; 2]], fs: u32, path: &Path, start_s: f64, end_s: f64, crossfade_s: f64) {
    let bgm = match read_wav_stereo(path, fs) { Ok(d) => d, Err(_) => return };
    if bgm.is_empty() { return; }
    let start_i = (start_s * fs as f64) as usize;
    let end_i   = ((end_s * fs as f64) as usize).min(out.len());
    let need = end_i.saturating_sub(start_i);
    if need == 0 { return; }
    // ループ展開
    let bgm_gain = 0.3f32;
    for i in 0..need {
        let src = &bgm[i % bgm.len()];
        let idx = start_i + i;
        if idx < out.len() { out[idx][0] += src[0] * bgm_gain; out[idx][1] += src[1] * bgm_gain; }
    }
}

fn mix_se(events: &[TimelineEvent], out: &mut [[f32; 2]], fs: u32, fade_s: f64) {
    for e in events.iter().filter(|e| e.event_type == EventType::Se) {
        let path = match &e.path { Some(p) => p, None => continue };
        let mut data = match read_wav_stereo(path, fs) { Ok(d) => d, Err(_) => continue };
        // フェードアウト
        let fade_n = (fade_s * fs as f64) as usize;
        let len = data.len();
        for (i, s) in data.iter_mut().enumerate().rev().take(fade_n) {
            let t = (len - 1 - i) as f32 / fade_n as f32;
            s[0] *= t; s[1] *= t;
        }
        let start = (e.start_ms / 1000.0 * fs as f64) as usize;
        for (i, s) in data.iter().enumerate() {
            let idx = start + i;
            if idx < out.len() { out[idx][0] += s[0]; out[idx][1] += s[1]; }
        }
    }
}
```

- [ ] **Step 5: lib.rs 更新**

```rust
// crates/s2v-export/src/lib.rs
pub mod srt;
pub mod fcpxml;
pub mod mix;

use std::path::Path;
use s2v_core::{timeline::TimelineEvent, Config};
use anyhow::Result;

pub struct Exporter<'a> {
    pub events: &'a [TimelineEvent],
    pub output_dir: &'a Path,
    pub config: &'a Config,
}

impl<'a> Exporter<'a> {
    pub fn export_all(&self) -> Result<()> {
        srt::generate_srt(self.events, self.output_dir)?;
        fcpxml::generate_fcpxml(self.events, self.output_dir, self.config.bgm.crossfade_s)?;
        mix::generate_combined_audio(
            self.events, self.output_dir,
            self.config.audio.sample_rate,
            self.config.bgm.se_fade_out_s,
            self.config.bgm.crossfade_s,
        )?;
        Ok(())
    }
}
```

- [ ] **Step 6: テスト PASS**

```
cargo test -p s2v-export
```

- [ ] **Step 7: コミット**

```
git add crates/s2v-export/
git commit -m "feat(export): implement SRT, FCPXML, and WAV mix exporter"
```

---

## Task 14: Producer (src/lib.rs)

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Producer 実装**

```rust
// src/lib.rs
use std::{collections::{HashMap, HashSet}, path::{Path, PathBuf}, sync::Arc};
use anyhow::Result;
use rayon::prelude::*;
use s2v_core::{
    Cast, Config,
    parser::{ScriptItem, ScriptCommand, Scene},
    timeline::{TimelineProcessor, TimelineEvent},
};
use s2v_engines::EngineManager;
use s2v_audio::processor::AudioProcessor;
use s2v_export::Exporter;

pub struct Producer {
    engine_manager: Arc<EngineManager>,
    audio_processor: Arc<AudioProcessor>,
    config: Config,
    project_dir: PathBuf,
    file_counter: std::sync::atomic::AtomicU32,
}

struct SynthTask {
    item_id: usize,
    cast: Cast,
    final_path: PathBuf,
    raw_path: PathBuf,
    scene_config: s2v_core::parser::SceneConfig,
    text: String,
    display_text: String,
    cast_name: String,
}

impl Producer {
    pub fn new(config: Config, project_dir: PathBuf) -> Self {
        let audio_processor = Arc::new(AudioProcessor::new(config.audio.clone()));
        let engine_manager = Arc::new(EngineManager::from_config(&config));
        Self { engine_manager, audio_processor, config, project_dir, file_counter: std::sync::atomic::AtomicU32::new(1) }
    }

    pub async fn produce(
        &self,
        scenes: &[Scene],
        casts: &HashMap<String, Cast>,
        pause_config: s2v_core::parser::PauseConfig,
        asset_config: s2v_core::parser::AssetConfig,
    ) -> Result<Vec<TimelineEvent>> {
        let audio_dir = self.project_dir.join("audio");
        std::fs::create_dir_all(&audio_dir)?;

        // Phase 1: 事前割当
        let mut task_map: HashMap<usize, SynthTask> = HashMap::new();
        let mut required_engines = HashSet::new();

        for scene in scenes {
            for (item_id, item) in scene.items.iter().enumerate().map(|(i, item)| (i + scene.name.len(), item)) {
                if let ScriptItem::Speech { cast_name, text, display_text, offset_params, scene_config } = item {
                    let cast = match casts.get(cast_name) { Some(c) => c, None => continue };
                    required_engines.insert(cast.engine_type.clone());
                    let effective = cast.with_offsets(offset_params);
                    let n = self.file_counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let final_path = audio_dir.join(format!("voice_{n:04}.wav"));
                    let raw_path   = audio_dir.join(format!("voice_{n:04}_raw.wav"));
                    task_map.insert(
                        item as *const _ as usize,
                        SynthTask { item_id, cast: effective, final_path, raw_path,
                                    scene_config: scene_config.clone(),
                                    text: text.clone(), display_text: display_text.clone(),
                                    cast_name: cast_name.clone() },
                    );
                }
            }
        }

        // エンジン起動
        self.engine_manager.activate_required(&required_engines).await?;

        // IR プリウォーム
        let room_sizes: Vec<f64> = task_map.values().map(|t| {
            t.cast.params.get("room_size").and_then(|v| v.as_f64())
                .or(t.scene_config.room_size)
                .unwrap_or(self.config.audio.room_size)
        }).collect();
        self.audio_processor.prewarm_ir_cache(&room_sizes);

        // Phase 2: 並列合成 + 音響処理
        let tasks: Vec<_> = task_map.values().collect();
        let results: Vec<(usize, f64)> = {
            let futs: Vec<_> = tasks.iter().map(|task| {
                let em = Arc::clone(&self.engine_manager);
                let ap = Arc::clone(&self.audio_processor);
                let text = task.text.clone();
                let cast = task.cast.clone();
                let raw  = task.raw_path.clone();
                let fin  = task.final_path.clone();
                let scene = task.scene_config.clone();
                let fs = self.config.audio.sample_rate;
                let ptr = *task as *const SynthTask as usize;
                tokio::spawn(async move {
                    let r = em.synthesize(&text, &cast, &raw).await;
                    if r.is_err() { return (ptr, 0.0); }
                    // 音響処理はブロッキングスレッドで
                    let dur = tokio::task::spawn_blocking(move || {
                        process_audio(&ap, &raw, &fin, &cast, &scene, fs)
                    }).await.unwrap_or(0.0);
                    (ptr, dur)
                })
            }).collect();
            let mut out = Vec::new();
            for f in futs { if let Ok(r) = f.await { out.push(r); } }
            out
        };

        let mut duration_map: HashMap<usize, f64> = results.into_iter().collect();

        // Phase 3: タイムライン構築
        let mut tl = TimelineProcessor::new(pause_config.clone());
        let mut last_cast: Option<String> = None;

        for scene in scenes {
            let items = &scene.items;
            let mut i = 0;
            while i < items.len() {
                let item = &items[i];
                match item {
                    ScriptItem::Command(ScriptCommand::Parallel(n)) => {
                        let anchor = tl.current_ms;
                        let mut max_occ = 0.0f64;
                        for pi in i+1..=(i+n).min(items.len()-1) {
                            let ptr = &items[pi] as *const _ as usize;
                            if let Some(task) = task_map.get(&ptr) {
                                let dur = *duration_map.get(&ptr).unwrap_or(&0.0);
                                tl.register_audio(task.final_path.clone(), dur, anchor, task.text.clone(), task.display_text.clone(), task.cast_name.clone());
                                max_occ = max_occ.max(dur);
                            }
                        }
                        tl.advance_after_parallel(anchor, max_occ, tl.sentence_pause_ms());
                        last_cast = None;
                        i += 1 + n;
                        continue;
                    }
                    ScriptItem::Speech { cast_name, .. } => {
                        let ptr = item as *const _ as usize;
                        if let Some(task) = task_map.get(&ptr) {
                            let dur = *duration_map.get(&ptr).unwrap_or(&0.0);
                            let pause = if last_cast.as_deref().map(|l| l != cast_name).unwrap_or(false) {
                                tl.cast_pause_ms()
                            } else { tl.sentence_pause_ms() };
                            tl.register_audio(task.final_path.clone(), dur, tl.current_ms, task.text.clone(), task.display_text.clone(), cast_name.clone());
                            tl.advance_after_speech(dur, pause);
                            last_cast = Some(cast_name.clone());
                        }
                    }
                    ScriptItem::Command(cmd) => {
                        match cmd {
                            ScriptCommand::Pause(ms)      => tl.advance_pause(*ms),
                            ScriptCommand::Paragraph       => tl.advance_paragraph(),
                            ScriptCommand::BgmStart(fname) => {
                                let p = resolve_asset(fname, &asset_config.bgm_dir);
                                tl.register_bgm(p);
                            }
                            ScriptCommand::BgmStop => tl.register_bgm_stop(),
                            ScriptCommand::Se(fname) => {
                                let p = resolve_asset(fname, &asset_config.se_dir);
                                tl.register_se(p);
                            }
                            _ => {}
                        }
                    }
                }
                i += 1;
            }
        }
        Ok(tl.into_events())
    }
}

fn process_audio(ap: &AudioProcessor, raw: &Path, fin: &Path, cast: &Cast, scene: &s2v_core::parser::SceneConfig, fs: u32) -> f64 {
    let result = (|| -> Result<f64> {
        let mut reader = hound::WavReader::open(raw)?;
        let spec = reader.spec();
        let src_rate = spec.sample_rate;
        let samples: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Int   => reader.samples::<i16>().map(|s| Ok(s? as f32 / 32768.0)).collect::<Result<_,hound::Error>>()?,
            hound::SampleFormat::Float => reader.samples::<f32>().collect::<Result<_,hound::Error>>()?,
        };
        let stereo = ap.process(&samples, src_rate, cast, scene);
        let wav_spec = hound::WavSpec { channels: 2, sample_rate: fs, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
        let mut writer = hound::WavWriter::create(fin, wav_spec)?;
        for s in &stereo { writer.write_sample((s[0]*32767.0) as i16)?; writer.write_sample((s[1]*32767.0) as i16)?; }
        writer.finalize()?;
        let dur_ms = stereo.len() as f64 / fs as f64 * 1000.0;
        Ok(dur_ms)
    })();
    if let Err(e) = std::fs::remove_file(raw) { /* raw は消せなくてもOK */ }
    result.unwrap_or_else(|e| { tracing::error!("audio process failed: {e}"); 0.0 })
}

fn resolve_asset(filename: &str, dir: &str) -> PathBuf {
    if dir.is_empty() || Path::new(filename).is_absolute() { PathBuf::from(filename) }
    else { Path::new(dir).join(filename) }
}
```

`src/lib.rs` の `[dependencies]` に `hound = "3"` を追加（root Cargo.toml）。

- [ ] **Step 2: ビルド確認**

```
cargo build
```
Expected: PASS（警告は無視）

- [ ] **Step 3: コミット**

```
git add src/lib.rs Cargo.toml
git commit -m "feat: implement Producer with 3-phase pipeline"
```

---

## Task 15: CLI (src/main.rs)

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: main.rs 実装**

```rust
// src/main.rs
use anyhow::Result;
use clap::Parser;
use std::path::{Path, PathBuf};
use tracing_subscriber::{EnvFilter, fmt};

#[derive(Parser)]
#[command(name = "script2voice", about = "台本テキストから音声・タイムラインを生成")]
struct Args {
    /// 台本ファイルのパス
    script: PathBuf,
    /// 設定ファイルパス
    #[arg(long, default_value = "config.toml")]
    config: PathBuf,
    /// ログレベル (trace/debug/info/warn/error)
    #[arg(long, default_value = "info")]
    log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // ロギング設定
    let filter = EnvFilter::try_new(&args.log_level).unwrap_or_else(|_| EnvFilter::new("info"));
    fmt().with_env_filter(filter).init();

    let config = s2v_core::Config::load(&args.config)?;
    let abs_script = args.script.canonicalize()?;
    let project_name = abs_script.file_stem().and_then(|s| s.to_str()).unwrap_or("output");
    let project_dir = abs_script.parent().unwrap_or(Path::new(".")).join(project_name);
    std::fs::create_dir_all(&project_dir)?;

    // ファイルログ設定
    let log_path = project_dir.join("process.log");
    let log_file = std::fs::File::create(&log_path)?;
    // (簡易: stderr + ファイル両方に出力。本番はtracing-appenderを使用可)
    tracing::info!("--- Project: {project_name} ---");
    tracing::info!("Output: {}", project_dir.display());

    // 台本解析
    let mut parser = s2v_core::parser::ScriptParser::new();
    let scenes = parser.parse_file(&abs_script)?;
    tracing::info!("{} scenes parsed", scenes.len());

    // 製造
    let producer = script2voice::Producer::new(config.clone(), project_dir.clone());
    let events = producer.produce(
        &scenes,
        &parser.casts,
        parser.pause_config.clone(),
        parser.asset_config.clone(),
    ).await?;

    // エクスポート
    let exporter = s2v_export::Exporter { events: &events, output_dir: &project_dir, config: &config };
    exporter.export_all()?;

    tracing::info!("--- Done ---");
    Ok(())
}
```

- [ ] **Step 2: ビルド確認**

```
cargo build --release
```
Expected: `target/release/script2voice.exe` が生成される

- [ ] **Step 3: コミット**

```
git add src/main.rs
git commit -m "feat: implement CLI with clap and tracing"
```

---

## Task 16: 統合テスト

**Files:**
- Create: `tests/integration_test.rs` (または `test/` 台本ファイルを使用)

- [ ] **Step 1: サンプル台本を用意**

```
# test/sample.txt
@pause
sentence 300
cast 200
paragraph 1000

@cast
ナレーター:narrator:default,xtts

@scene テスト

@script
ナレーター:これは統合テストです。
#pause 200
ナレーター:二行目のテストです。
```

- [ ] **Step 2: XTTS サーバーが動いている状態でE2Eテスト実行**

```
cargo run -- test/sample.txt --config config.toml
```

Expected:
```
[INFO] --- Project: sample ---
[INFO] [xtts] activated, N speakers
[INFO] [Success] voice_0001.wav (Dur: XXXms ...)
[INFO] [Success] voice_0002.wav (Dur: XXXms ...)
[INFO] SRT: .../sample/timeline/subtitles.srt
[INFO] FCPXML: .../sample/timeline/timeline.fcpxml
[INFO] Mixed WAV: .../sample/full_dialogue.wav
[INFO] --- Done ---
```

出力確認:
```
ls test/sample/audio/
ls test/sample/timeline/
```

- [ ] **Step 3: 最終コミット**

```
git add test/ tests/
git commit -m "test: add sample script for integration test"
```

---

## 注意事項

- `Producer` の `item as *const _ as usize` によるポインタキーは、各 ScriptItem が Vec 内で安定したアドレスを持つ前提。Phase 1 と Phase 3 で同じ `scenes` 参照を使うこと。
- `s2v-audio::processor` の `Geo` 構造体は関数内プライベート struct として定義しているが、コンパイラが外部公開を求める場合は `pub(crate)` に変更。
- Beads でタスク管理する場合: `bd create --title="Task N: XXX" --type=task` で各タスクを登録し、`bd update <id> --claim` で着手、`bd close <id>` で完了。
