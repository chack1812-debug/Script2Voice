# 台本 @cast/@scene 自由記述拡張 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `crates/s2v-core`のパーサーを拡張し、`@cast`セクションに役の外見描写、`@scene`セクションに場面の情景描写を自由記述で埋め込めるようにする（画像/動画生成プロンプト作成の入力として台本一本で完結させるため）。

**Architecture:** `ScriptParser`に「収集中のキャスト」を追跡する状態（`pending_cast_name`/`pending_cast_appearance`）を追加し、`@cast`セクション内の空行をキャスト間の区切りとして扱う状態機械にする。`@scene`セクションはヘッダー行の後、次の`@`セクションが現れるまでの非空行を`SceneConfig.description`にそのまま蓄積する（状態機械は不要、単純な追記）。取得したテキストは`Cast.appearance: Option<String>`・`SceneConfig.description: Option<String>`という新規フィールドに格納する。

**Tech Stack:** Rust (edition 2021想定、既存workspaceに合わせる), `cargo test`。追加の外部クレート依存なし。

## Global Constraints

- 既存の全テスト（`crates/s2v-core`および`Cast`/`SceneConfig`を参照する`s2v-engines`/`s2v-audio`/`s2v-gui`）が引き続き全てパスすること（後方互換必須）。
- 自由記述を含まない既存の台本ファイルの解析結果（`Scene`/`Cast`の値）は今回の変更前後で完全に同一であること。
- 承認済み設計書 `docs/superpowers/specs/2026-07-02-script-visual-description-extension-design.md` の仕様に従うこと。台本仕様書（`台本仕様.txt`）は本セッション内で既に更新済み（`@scene`は「4. 情景描写（自由記述）」、`@cast`は「外見描写（自由記述）」として追記済み）。

---

### Task 1: `Cast`に外見描写フィールド`appearance`を追加する

**Files:**
- Modify: `crates/s2v-core/src/cast.rs:6-21`（構造体定義）, `crates/s2v-core/src/cast.rs:62-79`（`base_cast()`テストヘルパー）
- Modify: `crates/s2v-core/src/parser.rs:194-198`（`parse_cast_line`内の`Cast`構築）
- Modify: `crates/s2v-core/src/types.rs:178-188`（`scene_accepts_casts`テスト内の`Cast`構築）
- Modify: `crates/s2v-engines/src/http_engine.rs:195-208`, `crates/s2v-engines/src/engine.rs:125-135`, `crates/s2v-engines/src/xtts_engine.rs:161-171`（各`dummy_cast()`テストヘルパー）
- Modify: `crates/s2v-audio/src/processor.rs:255-265`（`dummy_cast()`テストヘルパー）
- Modify: `crates/s2v-gui/src/scene_line.rs:84-94`（`to_cast()`、GUI用の合成Cast生成）

**Interfaces:**
- Produces: `Cast.appearance: Option<String>`（`#[serde(default)]`付き）。Task 3がこのフィールドに値を書き込む。

`Cast`にフィールドを追加すると、Rustは構造体リテラルで全フィールドを明示する箇所すべてに`appearance: None`（または値）を書き足さないとコンパイルが通らない（`Default`実装が無いため）。上記ファイルすべてが該当する。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/cast.rs`の`mod tests`内、`base_cast()`関数の直後に以下を追加する：

```rust
    #[test]
    fn base_cast_has_no_appearance_by_default() {
        let cast = base_cast();
        assert_eq!(cast.appearance, None);
    }

    #[test]
    fn appearance_field_stores_free_text() {
        let mut cast = base_cast();
        cast.appearance = Some("小柄で緑髪の元気なキャラクター。".to_string());
        assert_eq!(cast.appearance.as_deref(), Some("小柄で緑髪の元気なキャラクター。"));
    }
```

- [ ] **Step 2: コンパイルが失敗することを確認する**

Run: `cargo test -p s2v-core --lib cast:: -- --nocapture`
Expected: FAIL（`no field \`appearance\` on type \`Cast\`` というコンパイルエラー）

- [ ] **Step 3: `Cast`にフィールドを追加し、全構築箇所を更新する**

`crates/s2v-core/src/cast.rs:6-21`を以下に置き換える：

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Cast {
    pub name: String,
    pub speaker_name: String,
    pub engine_type: String,
    pub pan: f64,
    pub distance: f64,
    pub volume: f64,
    pub params: HashMap<String, Value>,
    /// 話者の床面からの絶対高さ[m]の基準値。None = 聴取者と同じ高さ。
    #[serde(default)]
    pub height: Option<f64>,
    /// 行内臨時パラメータで加算される、その行限定の高さオフセット[m]。
    #[serde(default)]
    pub height_offset: f64,
    /// 役の外見・特徴の自由記述（画像/動画生成プロンプト作成用）。
    /// 台本の`@cast`セクションで定義行の次に書かれた自由記述行から設定される。
    #[serde(default)]
    pub appearance: Option<String>,
}
```

`crates/s2v-core/src/cast.rs`の`base_cast()`（62-79行目付近）の`height_offset: 0.0,`の直後に追加：

```rust
            height: None,
            height_offset: 0.0,
            appearance: None,
        }
    }
```

`crates/s2v-core/src/parser.rs:196`（`parse_cast_line`内）を以下に置き換える：

```rust
            Cast { name, speaker_name, engine_type, pan, distance, volume, params, height, height_offset: 0.0, appearance: None },
```

`crates/s2v-core/src/types.rs:178-188`（`scene_accepts_casts`テスト）の`Cast { ... }`の`height_offset: 0.0,`の直後に追加：

```rust
            height: None,
            height_offset: 0.0,
            appearance: None,
        };
```

`crates/s2v-engines/src/http_engine.rs`・`crates/s2v-engines/src/engine.rs`・`crates/s2v-engines/src/xtts_engine.rs`の各`dummy_cast()`、および`crates/s2v-audio/src/processor.rs`の`dummy_cast()`について、それぞれの`height_offset: 0.0,`の直後に`appearance: None,`を追加する。

`crates/s2v-gui/src/scene_line.rs:84-94`（`to_cast()`）の`height_offset: 0.0,`の直後に`appearance: None,`を追加する。

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-core --lib cast::`
Expected: PASS（`base_cast_has_no_appearance_by_default`、`appearance_field_stores_free_text`含む全件）

Run: `cargo build --workspace`
Expected: 成功（全クレートがコンパイルエラーなし）

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/cast.rs crates/s2v-core/src/parser.rs crates/s2v-core/src/types.rs crates/s2v-engines/src/http_engine.rs crates/s2v-engines/src/engine.rs crates/s2v-engines/src/xtts_engine.rs crates/s2v-audio/src/processor.rs crates/s2v-gui/src/scene_line.rs
git commit -m "feat(core): add Cast.appearance field for visual description"
```

---

### Task 2: `SceneConfig`に情景描写フィールド`description`を追加する

**Files:**
- Modify: `crates/s2v-core/src/types.rs:7-45`（構造体定義と`SceneConfig::new`）
- Modify: `crates/s2v-core/src/types.rs:108-121`（`scene_config_defaults`テスト、アサーション追加）

**Interfaces:**
- Consumes: なし（Task 1とは独立）
- Produces: `SceneConfig.description: Option<String>`（`#[serde(default)]`付き）。`SceneConfig::new()`経由で`None`初期化される。Task 4がこのフィールドに値を書き込む。

`crates/s2v-core/src/parser.rs:119-143`の`parse_scene_header`や、`s2v-audio`/`s2v-gui`側の`SceneConfig { ... }`構築箇所は全て`..SceneConfig::new(name)`のスプレッド構文を使っているため、`SceneConfig::new()`さえ更新すれば自動的に追従する（個別修正は不要）。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/types.rs`の`scene_config_defaults`テスト（108-121行目）の末尾（`assert_eq!(sc.listener_z, None);`の直後、閉じ`}`の前）に1行追加する：

```rust
        assert_eq!(sc.listener_z, None);
        assert_eq!(sc.description, None);
    }
```

続けて同テストモジュールに新規テストを追加する：

```rust
    #[test]
    fn scene_config_can_hold_description() {
        let sc = SceneConfig { description: Some("放課後の静かな教室。".to_string()), ..SceneConfig::new("教室") };
        assert_eq!(sc.description.as_deref(), Some("放課後の静かな教室。"));
    }
```

- [ ] **Step 2: コンパイルが失敗することを確認する**

Run: `cargo test -p s2v-core --lib types:: -- --nocapture`
Expected: FAIL（`no field \`description\` on type \`SceneConfig\`` というコンパイルエラー）

- [ ] **Step 3: `SceneConfig`にフィールドを追加する**

`crates/s2v-core/src/types.rs:7-29`を以下に置き換える：

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SceneConfig {
    pub name: String,
    /// 省略時は `None`。実効値への解決は処理時に AudioConfig の値へフォールバックする
    /// (Python版 audio_processor.py の `getattr(config, 'ROOM_SIZE'/'REVERB_WET', ...)` 相当)。
    pub room_size: Option<f64>,
    pub reverb_wet: Option<f64>,
    /// 部屋寸法[m]。3つすべて指定されたとき room_size より優先される。
    #[serde(default)]
    pub room_w: Option<f64>,
    #[serde(default)]
    pub room_d: Option<f64>,
    #[serde(default)]
    pub room_h: Option<f64>,
    /// 聴取者(マイクペア中心)の部屋中央からのオフセット[m]。省略時は config の listener_offset。
    #[serde(default)]
    pub listener_dx: Option<f64>,
    #[serde(default)]
    pub listener_dy: Option<f64>,
    /// 聴取者(マイクペア中心)の床面からの絶対高さ[m]。省略時は config の ear_height。
    #[serde(default)]
    pub listener_z: Option<f64>,
    /// 場面の情景・雰囲気の自由記述（画像/動画生成プロンプト作成用）。
    /// 台本の`@scene`ヘッダー行の後に書かれた自由記述行から設定される。
    #[serde(default)]
    pub description: Option<String>,
}

impl SceneConfig {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            room_size: None,
            reverb_wet: None,
            room_w: None,
            room_d: None,
            room_h: None,
            listener_dx: None,
            listener_dy: None,
            listener_z: None,
            description: None,
        }
    }
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-core --lib types::`
Expected: PASS（`scene_config_defaults`、`scene_config_can_hold_description`含む全件）

Run: `cargo build --workspace`
Expected: 成功

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/types.rs
git commit -m "feat(core): add SceneConfig.description field for scene setting text"
```

---

### Task 3: `@cast`セクションを自由記述対応の状態機械にする

**Files:**
- Modify: `crates/s2v-core/src/parser.rs:16-101`（`ScriptParser`構造体・`new()`・`parse_str`）
- Modify: `crates/s2v-core/src/parser.rs:168-198`（`parse_cast_line`）
- Test: `crates/s2v-core/src/parser.rs`（`mod tests`内に追加）

**Interfaces:**
- Consumes: `Cast.appearance: Option<String>`（Task 1で追加済み）
- Produces: `ScriptParser::flush_pending_cast(&mut self)`（内部メソッド、Task 4は使わない）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs`の`mod tests`内、`cast_line_parses_height_into_field`テストの直後に追加する：

```rust
    #[test]
    fn cast_appearance_collected_until_blank_line() {
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   ずんだもん:ずんだもん:ノーマル,voicevox,pan=0\n\
                   小柄で緑髪の元気なキャラクター。\n\
                   ずんだ餅のイメージカラーの服を着ている。\n\
                   \n\
                   @script\n\
                   ずんだもん:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        let cast = scenes[0].casts.get("ずんだもん").unwrap();
        assert_eq!(
            cast.appearance.as_deref(),
            Some("小柄で緑髪の元気なキャラクター。\nずんだ餅のイメージカラーの服を着ている。")
        );
    }

    #[test]
    fn cast_without_free_text_has_no_appearance() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let cast = scenes[0].casts.get("ずんだもん").unwrap();
        assert_eq!(cast.appearance, None);
    }

    #[test]
    fn cast_entries_without_blank_line_separator_merge_into_appearance() {
        // 空行を挟まないと、次のキャスト定義行が前のキャストの自由記述として飲み込まれる(仕様どおりの制約)
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   A:話者A:ノーマル,voicevox,pan=0\n\
                   B:話者B:ノーマル,voicevox,pan=10\n\
                   \n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert!(scenes[0].casts.contains_key("A"));
        assert!(!scenes[0].casts.contains_key("B"));
        let cast_a = scenes[0].casts.get("A").unwrap();
        assert_eq!(
            cast_a.appearance.as_deref(),
            Some("B:話者B:ノーマル,voicevox,pan=10")
        );
    }

    #[test]
    fn cast_appearance_flushes_without_trailing_blank_line_before_next_section() {
        let src = "@scene テスト room_size=0.1\n\
                   @cast\n\
                   A:話者A:ノーマル,voicevox,pan=0\n\
                   眼鏡をかけた青年。\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        let cast = scenes[0].casts.get("A").unwrap();
        assert_eq!(cast.appearance.as_deref(), Some("眼鏡をかけた青年。"));
    }
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p s2v-core --lib parser::tests::cast_appearance -- --nocapture`
Expected: FAIL（`cast_appearance_collected_until_blank_line`等が、`appearance`が`None`のままでassertion失敗する。まだ状態機械を実装していないため）

- [ ] **Step 3: `ScriptParser`を状態機械化する**

`crates/s2v-core/src/parser.rs:16-31`（構造体定義と`new()`）を以下に置き換える：

```rust
pub struct ScriptParser {
    casts: HashMap<String, Cast>,
    pause_config: PauseConfig,
    asset_config: HashMap<String, String>,
    warnings: Vec<ParseWarning>,
    /// `@cast`セクションで定義行を読んだ直後から、空行または次のセクションが来るまでの間、
    /// 収集中のキャスト名を保持する（自由記述の宛先を追跡するための状態）。
    pending_cast_name: Option<String>,
    /// `pending_cast_name`が`Some`の間に集めた自由記述の行（順序どおり）。
    pending_cast_appearance: Vec<String>,
}

impl ScriptParser {
    pub fn new() -> Self {
        Self {
            casts: HashMap::new(),
            pause_config: PauseConfig::default(),
            asset_config: HashMap::new(),
            warnings: Vec::new(),
            pending_cast_name: None,
            pending_cast_appearance: Vec::new(),
        }
    }
```

`crates/s2v-core/src/parser.rs`の`parse_str`冒頭（`self.warnings.clear();`の直後）に2行追加する：

```rust
    pub fn parse_str(&mut self, text: &str) -> anyhow::Result<Vec<Scene>> {
        self.warnings.clear();
        self.pending_cast_name = None;
        self.pending_cast_appearance.clear();
        let mut scenes: Vec<Scene> = Vec::new();
```

`parse_str`内の空行判定（`if line.is_empty() { continue; }`）を以下に置き換える：

```rust
            if line.is_empty() {
                self.flush_pending_cast();
                continue;
            }
```

`parse_str`内の`if line.starts_with('@') {`の直後（`if line.starts_with("@scene") {`より前）に1行追加する：

```rust
            if line.starts_with('@') {
                self.flush_pending_cast();
                if line.starts_with("@scene") {
```

`parse_str`内の`match section { "@cast" => self.parse_cast_line(line), ... }`の`"@cast"`の腕を以下に置き換える：

```rust
                "@cast" => {
                    if self.pending_cast_name.is_some() {
                        self.pending_cast_appearance.push(line.to_string());
                    } else {
                        self.parse_cast_line(line);
                    }
                }
```

`parse_str`の末尾、`if let Some(mut s) = current_scene { ... }`の直前に1行追加する：

```rust
        self.flush_pending_cast();

        if let Some(mut s) = current_scene {
            s.casts = self.casts.clone();
            Self::fill_items_scene_config(&mut s);
            scenes.push(s);
        }

        Ok(scenes)
    }
```

`fill_items_scene_config`メソッドの直後に新規メソッドを追加する：

```rust
    /// 収集中のキャストの自由記述をバッファから確定させ、`Cast.appearance`へ書き込む。
    /// 何も収集していない場合は何もしない(空行・セクション境界のたびに無条件で呼んでよい)。
    fn flush_pending_cast(&mut self) {
        if let Some(name) = self.pending_cast_name.take() {
            if !self.pending_cast_appearance.is_empty() {
                if let Some(cast) = self.casts.get_mut(&name) {
                    cast.appearance = Some(self.pending_cast_appearance.join("\n"));
                }
            }
            self.pending_cast_appearance.clear();
        }
    }
```

`parse_cast_line`（168-198行目）を以下に置き換える：

```rust
    fn parse_cast_line(&mut self, line: &str) {
        let parts: Vec<&str> = line.splitn(3, ':').collect();
        if parts.len() < 3 {
            return;
        }
        let name = parts[0].trim().to_string();
        let speaker_name = parts[1].trim().to_string();
        let remain = parts[2].trim();

        let sub: Vec<&str> = remain.splitn(3, ',').collect();
        let style = sub.first().map(|s| s.trim()).unwrap_or("").to_string();
        let engine_type = sub.get(1).map(|s| s.trim()).unwrap_or("").to_string();
        let params_str = sub.get(2).copied().unwrap_or("");

        let mut raw = extract_kv_params(params_str);
        let pan = raw.remove("pan").unwrap_or(0.0);
        let distance = raw.remove("distance").unwrap_or(1.0);
        let volume = raw.remove("volume").unwrap_or(1.0);
        let height = raw.remove("height");

        let mut params: HashMap<String, Value> = raw
            .into_iter()
            .map(|(k, v)| (k, Value::from(v)))
            .collect();
        params.insert("style".to_string(), Value::String(style));

        let cast_key = name.clone();
        self.casts.insert(
            cast_key.clone(),
            Cast { name, speaker_name, engine_type, pan, distance, volume, params, height, height_offset: 0.0, appearance: None },
        );
        self.pending_cast_name = Some(cast_key);
    }
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-core --lib parser::`
Expected: PASS（新規4件を含め、`parser.rs`内の全テストがパス。既存の`unknown_cast_produces_warning_with_line_number`、`warnings_are_reset_per_parse`等の回帰も確認する）

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/parser.rs
git commit -m "feat(core): support free-text appearance description in @cast section"
```

---

### Task 4: `@scene`セクションに情景描写の蓄積を追加する

**Files:**
- Modify: `crates/s2v-core/src/parser.rs`（`parse_str`内の`match section`に`"@scene"`の腕を追加）
- Test: `crates/s2v-core/src/parser.rs`（`mod tests`内に追加）

**Interfaces:**
- Consumes: `SceneConfig.description: Option<String>`（Task 2で追加済み）
- Produces: なし（末端機能）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs`の`mod tests`内、Task 3で追加した`cast_appearance_flushes_without_trailing_blank_line_before_next_section`テストの直後に追加する：

```rust
    #[test]
    fn scene_description_collected_until_next_section() {
        let src = "@scene 教室 room_size=0.3\n\
                   放課後の静かな教室。窓から夕日が差し込んでいる。\n\
                   黒板には日直の名前が書かれている。\n\
                   @pause\n\
                   sentence 200\n\
                   @cast\n\
                   A:A:ノーマル,voicevox\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert_eq!(
            scenes[0].config.description.as_deref(),
            Some("放課後の静かな教室。窓から夕日が差し込んでいる。\n黒板には日直の名前が書かれている。")
        );
    }

    #[test]
    fn scene_without_free_text_has_no_description() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        assert_eq!(scenes[0].config.description, None);
    }

    #[test]
    fn scene_description_ignores_blank_lines_within_block() {
        let src = "@scene 教室 room_size=0.3\n\
                   一行目の描写。\n\
                   \n\
                   二行目の描写。\n\
                   @cast\n\
                   A:A:ノーマル,voicevox\n\
                   @script\n\
                   A:こんにちは\n";
        let scenes = ScriptParser::new().parse_str(src).unwrap();
        assert_eq!(
            scenes[0].config.description.as_deref(),
            Some("一行目の描写。\n二行目の描写。")
        );
    }
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p s2v-core --lib parser::tests::scene_description -- --nocapture`
Expected: FAIL（`description`が`None`のままでassertion失敗する）

- [ ] **Step 3: `@scene`本文蓄積を実装する**

`crates/s2v-core/src/parser.rs`の`match section { ... }`（Task 3で`"@cast"`の腕を書き換えた同じブロック）に、`"@script"`の腕の直前に`"@scene"`の腕を追加する：

```rust
                "@scene" => {
                    if let Some(ref mut s) = current_scene {
                        match s.config.description {
                            Some(ref mut desc) => {
                                desc.push('\n');
                                desc.push_str(line);
                            }
                            None => s.config.description = Some(line.to_string()),
                        }
                    }
                }
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-core --lib parser::`
Expected: PASS（`parser.rs`内の全テスト、Task 3の4件・Task 4の3件を含めて全件パス）

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/parser.rs
git commit -m "feat(core): support free-text setting description in @scene section"
```

---

### Task 5: ワークスペース全体の回帰確認

**Files:**
- なし（新規変更なし。確認のみ）

**Interfaces:**
- Consumes: Task 1〜4の全変更
- Produces: なし

- [ ] **Step 1: ワークスペース全体のテストを実行する**

Run: `cargo test --workspace`
Expected: PASS（`s2v-core`/`s2v-engines`/`s2v-audio`/`s2v-gui`含む全クレートの既存テスト・新規テストが全件パス。1件でも失敗したら原因を特定し、Task 1〜4の該当箇所を修正してから再実行する）

- [ ] **Step 2: リリースビルドの健全性を確認する**

Run: `cargo build --workspace --release`
Expected: 成功（警告があれば内容を確認し、今回の変更に起因するものであれば解消する）

- [ ] **Step 3: 実台本での動作確認（任意の手動確認）**

`scripts/Script2Voice紹介イベント.txt`等、既存の実台本ファイルに対して既存のCLIバイナリでパースが通ることを確認する（自由記述を含まないため、出力が今回の変更前と完全に一致することの確認）。

Run: 既存のCLI実行コマンド（プロジェクトのREADME/既存手順に従う）で該当台本を処理し、エラーが出ないこと・音声/SRT出力が変更前と一致することを目視確認する。

- [ ] **Step 4: コミット（変更があれば）**

Task 5はコード変更を伴わない確認作業のため、通常はコミット不要。もしStep 1〜2で問題が見つかり修正した場合のみ、修正内容に応じたコミットメッセージでコミットする。
