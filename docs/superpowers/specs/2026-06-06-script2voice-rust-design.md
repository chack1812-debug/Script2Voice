# Script2Voice Rust版 設計書

**日付:** 2026-06-06  
**対象プロジェクト:** D:\UDS\Script2Voice-Rust版  
**参照元:** D:\UDS\Script2Voice (Python版)

---

## 1. 概要

Script2Voice の Python 版を Rust で再構築する。台本テキストファイルを入力として受け取り、複数の音声合成エンジン（VOICEVOX・AivisSpeech・XTTS）で合成し、空間音響処理を施したうえで SRT 字幕・FCPXML タイムライン・混合 WAV を出力する CLI ツール。

### 制約・前提
- XTTS は HTTP サーバー固定（ローカル直接呼び出しなし）
- VOICEVOX・AivisSpeech も同様に HTTP API 経由
- 初期実装は CLI のみ。コア処理をライブラリクレートに分離し、後から GUI を追加可能な構造にする
- 台本フォーマットは Python 版と完全互換
- Beads (`bd`) でタスク管理、Obsidian で台本・設計ドキュメント管理

---

## 2. アーキテクチャ

### 2.1 Cargo Workspace 構成

```
script2voice/                    ← Cargo workspace ルート
├── Cargo.toml                   workspace メンバー定義
├── config.toml                  実行設定 (Python版 config.py 相当)
├── src/
│   └── main.rs                  CLI エントリポイント (clap)
├── crates/
│   ├── s2v-core/                パーサー・タイムライン・共通型
│   ├── s2v-engines/             Engine trait + HTTP エンジン実装
│   ├── s2v-audio/               DSP 音響処理
│   └── s2v-export/              出力生成 (SRT/FCPXML/WAV)
├── docs/                        設計ドキュメント
└── gui/                         (将来追加)
```

### 2.2 クレート依存関係

```
main (CLI)
  └── s2v-core
  └── s2v-engines  →  s2v-core
  └── s2v-audio    →  s2v-core
  └── s2v-export   →  s2v-core, s2v-audio

gui/ (将来)
  └── s2v-core
  └── s2v-engines
  └── s2v-audio
  └── s2v-export
```

---

## 3. 処理パイプライン

Python 版と同じ 3 フェーズ構成を踏襲する。

```
台本ファイル (.txt)
  ↓ ScriptParser::parse_file()
  Vec<Scene>  (Scene ごとに ScriptItem のリスト)
  ↓ Producer::phase1_assign()
  TaskMap: HashMap<usize, SynthTask>  (ファイルパス事前割当)
  ↓ Producer::phase2_synthesize_and_process()  [tokio::join_all + rayon]
  各 SynthTask に duration_ms が確定
  ↓ Producer::phase3_build_timeline()
  Vec<TimelineEvent>
  ↓ Exporter::export_all()
  subtitles.srt / timeline.fcpxml / full_dialogue.wav / process.log
```

---

## 4. s2v-core クレート

### 4.1 台本パーサー

Python 版 `core/parser.py` に相当。入力フォーマットは完全互換。

**セクション:**
- `@scene <name> [room_size=N] [reverb_wet=N]`
- `@pause` — sentence/cast/paragraph の無音時間設定 (ms)
- `@asset` — bgm_dir / se_dir 設定
- `@cast` — `役名:話者名:スタイル,エンジン,パラメータ...`
- `@script` — 台詞行・コマンド行

**ScriptItem:**
```rust
pub enum ScriptItem {
    Speech {
        cast_name: String,
        text: String,           // 合成用テキスト (ルビ展開済み)
        display_text: String,   // 字幕用テキスト
        offset_params: HashMap<String, f64>,
        scene_config: SceneConfig,
    },
    Command(ScriptCommand),
}

pub enum ScriptCommand {
    Pause(f64),          // ms (#pause N)
    Paragraph,           // (#paragraph)
    BgmStart(String),   // ファイル名 (#bgm_start)
    BgmStop,             // (#bgm_stop)
    Se(String),          // (#se)
    Parallel(usize),    // 次 N 行を同時発声 (数字のみ行)
}
```

**Cast:**
```rust
#[derive(Clone)]
pub struct Cast {
    pub name: String,
    pub speaker_name: String,
    pub engine_type: String,
    pub pan: f64,           // 角度 (度)
    pub distance: f64,      // 距離 (m)
    pub volume: f64,        // 音量倍率
    pub params: HashMap<String, Value>,  // style, speedScale 等
}

impl Cast {
    /// 臨時パラメータを適用した新 Cast を返す (Python 版 create_effective_cast 相当)
    pub fn with_offsets(&self, offsets: &HashMap<String, f64>) -> Self { ... }
}
```

### 4.2 タイムライン

```rust
pub struct TimelineEvent {
    pub event_type: EventType,  // Audio | BgmStart | BgmStop | Se
    pub start_ms: f64,
    pub duration_ms: f64,
    pub path: Option<PathBuf>,
    pub text: Option<String>,
    pub display_text: Option<String>,
    pub cast: Option<String>,
}

pub struct TimelineProcessor {
    pub current_ms: f64,
    events: Vec<TimelineEvent>,
    sentence_pause_ms: f64,
    cast_pause_ms: f64,
    paragraph_pause_ms: f64,
}
```

### 4.3 設定 (config.toml)

```toml
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
audio_process = 0   # 0 = CPU コア数を自動使用

[bgm]
crossfade_s = 3.0
se_fade_out_s = 0.05
```

---

## 5. s2v-engines クレート

### 5.1 Engine trait

```rust
use async_trait::async_trait;

#[async_trait]
pub trait Engine: Send + Sync {
    /// エンジン起動・接続確認。失敗時は Err を返す
    async fn activate(&self) -> anyhow::Result<()>;

    /// text を合成して output_path に PCM WAV として保存
    async fn synthesize(&self, text: &str, cast: &Cast, output: &Path) -> anyhow::Result<()>;

    /// 話者・スタイルの有効性確認（デフォルト: 常に true）
    fn is_cast_valid(&self, cast: &Cast) -> bool { true }
}
```

### 5.2 EngineManager

```rust
pub struct EngineManager {
    engines: HashMap<String, Arc<dyn Engine>>,
}

impl EngineManager {
    pub fn from_config(config: &Config) -> Self;
    pub fn get(&self, engine_type: &str) -> Option<Arc<dyn Engine>>;
    pub async fn activate_required(&self, types: &HashSet<String>) -> anyhow::Result<()>;
    pub async fn synthesize(&self, text: &str, cast: &Cast, out: &Path) -> anyhow::Result<()>;
}
```

### 5.3 HTTP エンジン実装

全エンジンで `reqwest::Client` を共有 (Arc)。タイムアウト設定は config から取得。

**VOICEVOX / AivisSpeech (`HttpEngine`):**
- `GET /speakers` でスピーカーキャッシュ取得
- `POST /audio_query` → `POST /synthesis` の 2 ステップ
- スタイル名 → スタイル ID 変換をキャッシュに基づいて行う

**XTTS (`XttsEngine`):**
- `GET /speakers` で話者キャッシュ取得
- `POST /get_tts_settings` → `POST /set_tts_settings` → `POST /tts_to_audio/`

### 5.4 並列制御

エンジン種別ごとの `tokio::Semaphore` で同時合成数を制限。`config.toml` の `[concurrency]` セクションから上限値を取得。

---

## 6. s2v-audio クレート

### 6.1 AudioProcessor

Python 版 `core/audio_processor.py` の完全 Rust 移植。

```rust
pub struct AudioProcessor {
    config: AudioConfig,
    ir_cache: Mutex<HashMap<OrderedFloat<f64>, [Vec<f32>; 2]>>,
}

impl AudioProcessor {
    pub fn prewarm_ir_cache(&self, room_sizes: &[f64]);
    pub fn process(&self, input: &[f32], cast: &Cast, scene: &SceneConfig) -> Vec<[f32; 2]>;
}
```

**処理ステップ（Python 版と同等）:**
1. リサンプリング: `rubato::FftFixedIn` (SIMD 最適化)
2. 正規化: ピーク正規化
3. 幾何学計算: ITD (到達時間差) / ILD (音量差) / マイク指向性パターン
4. 高域減衰 (空気吸収): `biquad` フィルター
5. 空間リバーブ: IR 生成 + FFT 畳み込み (`realfft::RealFftPlanner`)
6. リミッター: ピーク超過時の正規化

**並列処理:** `rayon::ThreadPool` で複数音声の処理を並列化。スレッド数は `config.concurrency.audio_process` から設定。

### 6.2 IR キャッシュ

`room_size` をキーとした `Mutex<HashMap<..., [Vec<f32>;2]>>` でリバーブ IR をキャッシュ。Phase 2 開始前に `prewarm_ir_cache()` でシングルスレッド事前計算し、並列処理中の競合書き込みを回避する（Python 版と同じ設計）。

---

## 7. s2v-export クレート

### 7.1 Exporter

```rust
pub struct Exporter<'a> {
    events: &'a [TimelineEvent],
    output_dir: &'a Path,
    config: &'a ExportConfig,
}

impl<'a> Exporter<'a> {
    pub fn generate_srt(&self) -> anyhow::Result<()>;
    pub fn generate_fcpxml(&self) -> anyhow::Result<()>;
    pub fn generate_combined_audio(&self) -> anyhow::Result<()>;
}
```

- **SRT:** `timeline/subtitles.srt` — Filmora 字幕インポート用
- **FCPXML:** `timeline/timeline.fcpxml` — FCPXML 1.8 形式、BGM クロスフェード・SE フェードアウト付き
- **WAV ミックス:** `full_dialogue.wav` — `hound` で読み書き、float32 バッファで加算ミックス

---

## 8. CLI (src/main.rs)

```
script2voice <台本ファイル> [オプション]

引数:
  <台本ファイル>    台本テキストファイルのパス

オプション:
  --config <path>  設定ファイルパス [デフォルト: config.toml]
  --log-level      tracing ログレベル [デフォルト: info]
  -h, --help
  -V, --version
```

出力先: `<台本ファイルのディレクトリ>/<台本名>/`  
ログ: `process.log` (ファイル) + stdout

---

## 9. エラー処理

- 各クレートは `thiserror` で固有エラー型を定義
- CLI エントリポイントは `anyhow::Result` で受け取り、エラーメッセージを stderr に出力
- エンジン起動失敗: 警告ログを出して継続（該当エンジンのキャストはスキップ）
- 合成失敗: 該当音声は `duration_ms = 0` として処理を継続
- ファイル I/O 失敗: `?` で伝播させて即座に中断

---

## 10. テスト方針

- `s2v-core`: パーサーのユニットテスト（各セクション、エッジケース）
- `s2v-audio`: DSP 関数のユニットテスト（ITD/ILD 計算、リバーブ係数）
- `s2v-engines`: HTTP モックサーバー (`wiremock`) を使った統合テスト
- `s2v-export`: SRT/FCPXML の文字列比較テスト
- エンドツーエンド: `test/` フォルダの台本ファイルを使った結合テスト

---

## 11. 主要クレート一覧

| 用途 | クレート | バージョン指定 |
|------|----------|---------------|
| 非同期ランタイム | `tokio` | features = ["full"] |
| HTTP クライアント | `reqwest` | features = ["json"] |
| Engine trait async | `async-trait` | |
| WAV 読み書き | `hound` | |
| リサンプリング | `rubato` | |
| FFT (リバーブ用) | `realfft` | |
| CPU 並列処理 | `rayon` | |
| CLI 引数 | `clap` | features = ["derive"] |
| 設定ファイル | `toml` + `serde` | |
| ログ | `tracing` + `tracing-subscriber` | |
| エラー処理 | `anyhow` + `thiserror` | |
| 浮動小数点 Map key | `ordered-float` | |
| テスト HTTP モック | `wiremock` | dev-dependencies |

---

## 12. Obsidian / Beads 連携

| ツール | 用途 |
|--------|------|
| **Obsidian** | 台本ファイルの執筆・管理、設計ノート、Rustクレート調査メモのクリッピング |
| **Beads (`bd`)** | 実装タスク追跡（`bd create/ready/close`）、セッション間の進捗維持 |
| **Git** | ソースコード・設計書・Beads issues の版管理 |

Beads ルール（CLAUDE.md に記載済み）:
- タスク追跡は `bd` を使用。TaskCreate / TodoWrite は使わない
- 永続知識は `bd remember` を使用。MEMORY.md は使わない
