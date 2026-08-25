# P0: ルビ半角パイプ化・役名区切り半角化・段落名称 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 台本のルビ区切りを半角パイプに、役名区切りを半角 `:` のみに変更し、`#paragraph` に名称を付けられるようにして、その名称を SRT と新設 `timeline/timeline.json` に流す。

**Architecture:** `s2v-core`（parser / timeline / types）→ `s2v-export`（SRT・timeline.json）→ `s2v-video`（マーカー名の読み取りのみ）の順に、既存の型を最小限拡張する。`ScriptCommand::Paragraph` を `Paragraph(Option<String>)` に、`TimelineEvent` に `name: Option<String>` を追加し、SRT の表示文字列 `[PARAGRAPH 名称]` は `TimelineProcessor` 側で組み立てる（`Exporter` は `display_text` をそのまま書き出す既存の作りを変えない）。カット結合・素材自動検索・キャラクター層は P1 以降で、本計画には含めない。

**Tech Stack:** Rust 2021 (workspace), regex, serde / serde_json, hound, anyhow, tracing / テストは標準の `#[test]` + tempfile / 変換スクリプトは Python 3（使い捨て）

**Spec:** `docs/superpowers/specs/2026-08-25-paragraph-cuts-and-character-layer-design.md`

## Global Constraints

- ルビ区切りは**半角パイプ `|` のみ**。全角 `｜` は区切りとして扱わない（引用符内にあれば警告のみ）。
- 役名区切りは**半角 `:` のみ**。全角 `：` の分岐は削除する。`@cast` 定義行は元から半角のみで、変更しない。
- SRT の段落マーカーは**大文字固定** `[PARAGRAPH]` / `[PARAGRAPH 名称]`。
- 旧ルビ記法 `'語:読み'` は展開しない（後方互換を持たせない）。
- 無名 `#paragraph` だけの台本は、SRT・音声とも**従来と完全に同一の出力**でなければならない（既存回の再ビルドを壊さない）。
- ユーザー向けメッセージ（警告・ログ）は日本語。コード内コメントも既存に倣い日本語。
- コミットはタスクごとに行う。**push はしない**（リポジトリの conservative プロファイル）。
- 各タスクの最後に `cargo test --workspace --all-targets` が緑であること。

---

### Task 1: ルビ区切りを半角パイプにする

**Files:**
- Modify: `crates/s2v-core/src/parser.rs`（`expand_ruby` 関数 344-360行付近、および同ファイル内テスト）

**Interfaces:**
- Consumes: なし（既存の `fn expand_ruby(text: &str) -> (String, String)` を書き換える）
- Produces: `fn expand_ruby(text: &str) -> (String, String)` — 第1要素が合成用テキスト（読み）、第2要素が表示用テキスト（語）。シグネチャは不変。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs` のテストモジュール内、既存の `ruby_notation_separates_text_and_display` を次の内容に**置き換える**（旧記法のテストは廃止するため）:

```rust
    #[test]
    fn ruby_notation_separates_text_and_display() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:'東京|とうきょう'に行く
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { text, display_text, .. } = &scenes[0].items[0] {
            assert_eq!(text, "とうきょうに行く");
            assert_eq!(display_text, "東京に行く");
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn ruby_word_may_contain_colon() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:'13:00|じゅうさんじ'に始めます
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { text, display_text, .. } = &scenes[0].items[0] {
            assert_eq!(text, "じゅうさんじに始めます");
            assert_eq!(display_text, "13:00に始めます");
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn legacy_colon_ruby_is_not_expanded() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:'東京:とうきょう'に行く
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { text, display_text, .. } = &scenes[0].items[0] {
            assert_eq!(text, "'東京:とうきょう'に行く");
            assert_eq!(display_text, "'東京:とうきょう'に行く");
        } else {
            panic!("expected speech");
        }
    }

    #[test]
    fn fullwidth_pipe_is_not_a_ruby_separator() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:'東京｜とうきょう'に行く
"#;
        let mut parser = ScriptParser::new();
        let scenes = parser.parse_str(script).unwrap();
        if let ScriptItem::Speech { text, .. } = &scenes[0].items[0] {
            assert_eq!(text, "'東京｜とうきょう'に行く");
        } else {
            panic!("expected speech");
        }
        assert!(
            parser.warnings.iter().any(|w| w.message.contains("｜")),
            "全角パイプの警告が出るべき: {:?}", parser.warnings
        );
    }
```

`parser.warnings` が private の場合は、テストが同じモジュール内（`mod tests` は同一ファイル）なのでそのままアクセスできる。フィールドが `pub` でなければ `parser.warnings` は同一クレート内から参照可能なことを確認する。

- [ ] **Step 2: テストが落ちることを確認**

Run: `cargo test -p s2v-core ruby -- --nocapture`
Expected: `ruby_notation_separates_text_and_display` / `ruby_word_may_contain_colon` / `legacy_colon_ruby_is_not_expanded` / `fullwidth_pipe_is_not_a_ruby_separator` が FAIL（旧正規表現がパイプを認識しない、旧コロン記法を展開してしまう、警告が無い）。

- [ ] **Step 3: `expand_ruby` を書き換える**

`crates/s2v-core/src/parser.rs` の `expand_ruby` を次で置き換える:

```rust
/// ルビ記法の正規表現。半角パイプのみを区切りとする（全角｜は対象外）。
fn ruby_re() -> &'static regex::Regex {
    static RE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"'([^'|]+?)\|([^'|]+?)'").expect("ルビ正規表現が不正"))
}

/// `'word|reading'` → (reading_text, display_text) に展開
fn expand_ruby(text: &str) -> (String, String) {
    let mut synthesis = text.to_string();
    let mut display = text.to_string();

    for cap in ruby_re().captures_iter(text) {
        let full = &cap[0];
        let word = &cap[1];
        let reading = &cap[2];
        synthesis = synthesis.replace(full, reading);
        display = display.replace(full, word);
    }
    (synthesis, display)
}
```

- [ ] **Step 4: 全角パイプの警告を追加**

`parse_script_line` の中、`let (text, display_text) = expand_ruby(raw_text);` の**直前**に次を挿入する:

```rust
        if raw_text.contains('｜') {
            self.warnings.push(ParseWarning {
                line_no,
                message: "台詞に全角「｜」が含まれます。ルビの区切りは半角「|」です（全角は区切りとして扱われません）".to_string(),
            });
        }
```

- [ ] **Step 5: テストが通ることを確認**

Run: `cargo test -p s2v-core ruby -- --nocapture`
Expected: PASS（4件すべて）

- [ ] **Step 6: ワークスペース全体のテスト**

Run: `cargo test --workspace --all-targets`
Expected: PASS。落ちる場合は旧ルビ記法を使っている他クレートのテスト（`src/lib.rs` の統合テスト等）が原因なので、その台本リテラルを `|` 記法へ直す。

- [ ] **Step 7: コミット**

```bash
git add crates/s2v-core/src/parser.rs
git commit -m "feat(parser)!: ルビの区切りを半角パイプに変更し旧コロン記法を廃止"
```

---

### Task 2: 役名区切りを半角 `:` のみにする

**Files:**
- Modify: `crates/s2v-core/src/parser.rs`（`parse_script_line` の 296-322行付近、および同ファイル内テスト）

**Interfaces:**
- Consumes: Task 1 の `expand_ruby`
- Produces: なし（`parse_script_line` の外部シグネチャは不変）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs` のテストモジュールに追加:

```rust
    #[test]
    fn fullwidth_role_separator_is_rejected_with_warning() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A：こんにちは
"#;
        let mut parser = ScriptParser::new();
        let scenes = parser.parse_str(script).unwrap();
        assert!(scenes[0].items.is_empty(), "全角：の行は台詞として扱わない");
        assert!(
            parser.warnings.iter().any(|w| w.message.contains("全角")),
            "全角：の警告が出るべき: {:?}", parser.warnings
        );
    }

    #[test]
    fn halfwidth_colon_in_dialogue_does_not_break_role_split() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:開始は'13:00|じゅうさんじ'です
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        if let ScriptItem::Speech { cast_name, text, .. } = &scenes[0].items[0] {
            assert_eq!(cast_name, "A");
            assert_eq!(text, "開始はじゅうさんじです");
        } else {
            panic!("expected speech");
        }
    }
```

- [ ] **Step 2: テストが落ちることを確認**

Run: `cargo test -p s2v-core role_separator halfwidth_colon -- --nocapture`
Expected: `fullwidth_role_separator_is_rejected_with_warning` が FAIL（現状は全角でも通ってしまい、警告も出ない）。

- [ ] **Step 3: 区切り判定を書き換える**

`parse_script_line` の以下のブロック（現状 296-307行付近）:

```rust
        // 台詞行: `役名(params):テキスト` or `役名:テキスト`
        let sep = if line.contains(':') {
            ':'
        } else if line.contains('：') {
            '：'
        } else {
            return None;
        };
        let (name_part, raw_text) = line.split_once(sep)?;
```

を、次で置き換える:

```rust
        // 台詞行: `役名(params):テキスト`（区切りは半角コロンのみ）
        let Some((name_part, raw_text)) = line.split_once(':') else {
            if line.contains('：') {
                self.warnings.push(ParseWarning {
                    line_no,
                    message: "行に全角「：」が使われています。役名の区切りは半角「:」です（この行は無視されます）".to_string(),
                });
            }
            return None;
        };
```

- [ ] **Step 4: 未定義キャスト警告に手掛かりを足す**

同関数内の未定義キャスト警告（現状 318-323行付近）を次に変更する:

```rust
        if !self.casts.contains_key(role) {
            let hint = if role.contains('：') {
                "（役名の区切りが全角「：」になっていないか確認してください）"
            } else {
                ""
            };
            self.warnings.push(ParseWarning {
                line_no,
                message: format!("キャスト「{role}」が未定義です（この行は無視されます）{hint}"),
            });
            return None;
        }
```

- [ ] **Step 5: テストが通ることを確認**

Run: `cargo test -p s2v-core -- --nocapture`
Expected: PASS

- [ ] **Step 6: 台本仕様の該当記述を直す**

`台本仕様.txt` の @script 節に、役名区切りが半角コロンのみである旨を追記する（既存の「書式	役名(臨時パラメータ):台詞…」の直後の行）:

```
		役名と台詞の区切りは半角の":"のみとする。全角の"："は区切りとして扱わない。
```

- [ ] **Step 7: コミット**

```bash
git add crates/s2v-core/src/parser.rs 台本仕様.txt
git commit -m "feat(parser)!: 役名の区切りを半角コロンのみにし全角コロン行を警告する"
```

---

### Task 3: `#paragraph` に名称を持たせる

**Files:**
- Modify: `crates/s2v-core/src/types.rs`（`ScriptCommand` 65-72行）
- Modify: `crates/s2v-core/src/parser.rs`（`parse_script_line` のコマンド分岐 287-295行、テスト）
- Modify: `src/lib.rs`（`ScriptCommand::Paragraph` の分岐 423-426行）

**Interfaces:**
- Consumes: なし
- Produces: `ScriptCommand::Paragraph(Option<String>)` — `None` は無名、`Some(name)` は trim 済みの名称。`]` を含む名称は `None` に落とす。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/parser.rs` のテストモジュールで、既存の `parses_paragraph_command` を次に置き換え、続く3件を追加する:

```rust
    #[test]
    fn parses_paragraph_command() {
        let scenes = ScriptParser::new().parse_str(SIMPLE_SCRIPT).unwrap();
        let found = scenes[0].items.iter().any(|i| {
            matches!(i, ScriptItem::Command(ScriptCommand::Paragraph(None)))
        });
        assert!(found);
    }

    #[test]
    fn parses_paragraph_with_name() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:こんにちは
#paragraph オープニング 前半
A:さようなら
"#;
        let scenes = ScriptParser::new().parse_str(script).unwrap();
        let name = scenes[0].items.iter().find_map(|i| match i {
            ScriptItem::Command(ScriptCommand::Paragraph(n)) => Some(n.clone()),
            _ => None,
        });
        assert_eq!(name, Some(Some("オープニング 前半".to_string())));
    }

    #[test]
    fn paragraph_name_with_bracket_is_dropped_with_warning() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:こんにちは
#paragraph 変な]名前
"#;
        let mut parser = ScriptParser::new();
        let scenes = parser.parse_str(script).unwrap();
        let name = scenes[0].items.iter().find_map(|i| match i {
            ScriptItem::Command(ScriptCommand::Paragraph(n)) => Some(n.clone()),
            _ => None,
        });
        assert_eq!(name, Some(None), "]を含む名称は無名として扱う");
        assert!(parser.warnings.iter().any(|w| w.message.contains("]")));
    }

    #[test]
    fn paragraph_name_with_invalid_filename_char_warns_but_is_kept() {
        let script = r#"
@scene テスト room_size=0.1

@cast
A:A:スタイル,voicevox

@script
A:こんにちは
#paragraph a/b
"#;
        let mut parser = ScriptParser::new();
        let scenes = parser.parse_str(script).unwrap();
        let name = scenes[0].items.iter().find_map(|i| match i {
            ScriptItem::Command(ScriptCommand::Paragraph(n)) => Some(n.clone()),
            _ => None,
        });
        assert_eq!(name, Some(Some("a/b".to_string())));
        assert!(parser.warnings.iter().any(|w| w.message.contains("ファイル名")));
    }
```

- [ ] **Step 2: テストが落ちることを確認**

Run: `cargo test -p s2v-core paragraph -- --nocapture`
Expected: コンパイルエラー（`ScriptCommand::Paragraph` は値を取らない）。

- [ ] **Step 3: enum を変更する**

`crates/s2v-core/src/types.rs`:

```rust
pub enum ScriptCommand {
    Pause(f64),
    /// 段落区切り。`Some(name)` は `#paragraph 名称` で付けられた名称。
    Paragraph(Option<String>),
    BgmStart(String),
    BgmStop,
    Se(String),
    Parallel(usize),
}
```

- [ ] **Step 4: パーサのコマンド分岐を実装する**

`crates/s2v-core/src/parser.rs` の `"paragraph" => Some(ScriptItem::Command(ScriptCommand::Paragraph)),` を次で置き換える:

```rust
                "paragraph" => {
                    let name = if arg.is_empty() { None } else { Some(arg.to_string()) };
                    let name = match name {
                        Some(n) if n.contains(']') => {
                            self.warnings.push(ParseWarning {
                                line_no,
                                message: format!("#paragraph の名称に「]」は使えません（名称なしとして扱います）: {n}"),
                            });
                            None
                        }
                        Some(n) => {
                            if n.contains(['\\', '/', ':', '*', '?', '"', '<', '>', '|']) {
                                self.warnings.push(ParseWarning {
                                    line_no,
                                    message: format!("#paragraph の名称にファイル名として使えない文字が含まれます（素材の自動検索が失敗します）: {n}"),
                                });
                            }
                            Some(n)
                        }
                        None => None,
                    };
                    Some(ScriptItem::Command(ScriptCommand::Paragraph(name)))
                }
```

- [ ] **Step 5: 呼び出し側を直す**

`src/lib.rs` の 423-426行付近:

```rust
                            ScriptCommand::Paragraph(name) => {
                                timeline.register_paragraph(name.clone());
                                timeline.advance_paragraph();
                            }
```

`register_paragraph` の引数追加は Task 4 で行うため、この時点では**先に Task 4 の Step 3 を実施してからビルドが通る**。順序を守るなら本タスクの Step 5 は「呼び出し箇所を `ScriptCommand::Paragraph(_name)` にパターンだけ合わせ、本体は `timeline.register_paragraph();` のまま」とし、Task 4 で名称を渡すよう変更する:

```rust
                            ScriptCommand::Paragraph(_name) => {
                                timeline.register_paragraph();
                                timeline.advance_paragraph();
                            }
```

- [ ] **Step 6: テストが通ることを確認**

Run: `cargo test -p s2v-core paragraph -- --nocapture`
Expected: PASS（4件）

Run: `cargo test --workspace --all-targets`
Expected: PASS

- [ ] **Step 7: コミット**

```bash
git add crates/s2v-core/src/types.rs crates/s2v-core/src/parser.rs src/lib.rs
git commit -m "feat(parser): #paragraph に名称を指定できるようにする"
```

---

### Task 4: 段落名を TimelineEvent と SRT に流す

**Files:**
- Modify: `crates/s2v-core/src/timeline.rs`（`TimelineEvent` 17-26行、`register_*` 各所、`register_paragraph` 122-131行、テスト）
- Modify: `src/lib.rs`（423-426行）
- Modify: `crates/s2v-export/src/exporter.rs`（テストヘルパ `make_paragraph_event` 674行付近、SRT テスト）

**Interfaces:**
- Consumes: `ScriptCommand::Paragraph(Option<String>)`（Task 3）
- Produces:
  - `TimelineEvent { …, pub name: Option<String> }` — 段落名。他イベントでは `None`。
  - `TimelineProcessor::register_paragraph(&mut self, name: Option<String>)`
  - SRT の段落エントリ本文: `[PARAGRAPH]`（無名）/ `[PARAGRAPH 名称]`

- [ ] **Step 1: 失敗するテストを書く（timeline）**

`crates/s2v-core/src/timeline.rs` のテストモジュールで、既存の `register_paragraph_uses_current_time_and_zero_duration` の呼び出しを `tp.register_paragraph(None);` に直し、次を追加する:

```rust
    #[test]
    fn register_paragraph_with_name_writes_named_marker() {
        let mut tp = TimelineProcessor::new(&pause_config());
        tp.current_ms = 1500.0;
        tp.register_paragraph(Some("オープニング".to_string()));
        let events = tp.get_events();
        assert_eq!(events[0].display_text.as_deref(), Some("[PARAGRAPH オープニング]"));
        assert_eq!(events[0].name.as_deref(), Some("オープニング"));
    }
```

（`pause_config()` は既存テストのヘルパ名に合わせる。既存テストが `PauseConfig { … }` を直に組んでいる場合はそれをコピーする。）

- [ ] **Step 2: 失敗するテストを書く（SRT）**

`crates/s2v-export/src/exporter.rs` のテストモジュールで、ヘルパ `make_paragraph_event` に名称引数を足し、名前付きのケースを追加する:

```rust
    fn make_paragraph_event(start_ms: f64) -> TimelineEvent {
        TimelineEvent {
            event_type: EventType::Paragraph,
            start_ms,
            duration_ms: 0.0,
            path: None,
            text: None,
            display_text: Some("[PARAGRAPH]".to_string()),
            cast: None,
            name: None,
        }
    }

    fn make_named_paragraph_event(start_ms: f64, name: &str) -> TimelineEvent {
        TimelineEvent {
            event_type: EventType::Paragraph,
            start_ms,
            duration_ms: 0.0,
            path: None,
            text: None,
            display_text: Some(format!("[PARAGRAPH {name}]")),
            cast: None,
            name: Some(name.to_string()),
        }
    }

    #[test]
    fn srt_writes_named_paragraph_marker() {
        let dir = tempfile::tempdir().unwrap();
        let events = vec![make_named_paragraph_event(1500.0, "オープニング")];
        Exporter::new(&events, dir.path(), 48000, BgmConfig::default())
            .generate_srt("")
            .unwrap();
        let content = std::fs::read_to_string(dir.path().join("timeline/subtitles.srt")).unwrap();
        assert!(content.contains("[PARAGRAPH オープニング]"), "実際の内容: {content}");
    }
```

（`BgmConfig::default()` と `Exporter::new` の引数は、同ファイル内の既存テスト（725行付近 `srt_includes_paragraph_markers_in_chronological_order_with_continuous_numbering`）の書き方をそのままコピーして合わせる。）

- [ ] **Step 3: テストが落ちることを確認**

Run: `cargo test -p s2v-core -p s2v-export paragraph -- --nocapture`
Expected: コンパイルエラー（`TimelineEvent` に `name` フィールドが無い / `register_paragraph` が引数を取らない）。

- [ ] **Step 4: `TimelineEvent` にフィールドを足す**

`crates/s2v-core/src/timeline.rs`:

```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TimelineEvent {
    pub event_type: EventType,
    pub start_ms: f64,
    pub duration_ms: f64,
    pub path: Option<PathBuf>,
    pub text: Option<String>,
    pub display_text: Option<String>,
    pub cast: Option<String>,
    /// 段落名（`#paragraph 名称`）。段落イベント以外は None。
    #[serde(default)]
    pub name: Option<String>,
}
```

`register_audio` / `register_bgm` / `register_bgm_stop` / `register_se` の各構造体リテラルに `name: None,` を追加する（4箇所）。

- [ ] **Step 5: `register_paragraph` を書き換える**

```rust
    pub fn register_paragraph(&mut self, name: Option<String>) {
        let display = match &name {
            Some(n) => format!("[PARAGRAPH {n}]"),
            None => "[PARAGRAPH]".to_string(),
        };
        self.events.push(TimelineEvent {
            event_type: EventType::Paragraph,
            start_ms: self.current_ms,
            duration_ms: 0.0,
            path: None,
            text: None,
            display_text: Some(display),
            cast: None,
            name,
        });
    }
```

- [ ] **Step 6: 呼び出し側で名称を渡す**

`src/lib.rs`:

```rust
                            ScriptCommand::Paragraph(name) => {
                                timeline.register_paragraph(name.clone());
                                timeline.advance_paragraph();
                            }
```

- [ ] **Step 7: テストが通ることを確認**

Run: `cargo test --workspace --all-targets`
Expected: PASS。`TimelineEvent` を組み立てている他のテストがあれば `name: None,` を追加する。

- [ ] **Step 8: コミット**

```bash
git add crates/s2v-core/src/timeline.rs crates/s2v-export/src/exporter.rs src/lib.rs
git commit -m "feat(export): SRTの段落マーカーに名称を出力する"
```

---

### Task 5: `timeline/timeline.json` を出力する

**Files:**
- Modify: `crates/s2v-export/Cargo.toml`（serde / serde_json 追加）
- Modify: `crates/s2v-export/src/exporter.rs`（`generate_timeline_json` 追加、テスト）
- Modify: `src/lib.rs`（237行付近のロック対象ファイル、447行付近の書き出し呼び出し）

**Interfaces:**
- Consumes: `TimelineEvent`（`name` 付き、Task 4）
- Produces: `Exporter::generate_timeline_json(&self, suffix: &str) -> anyhow::Result<()>` — `<project>/timeline/timeline.json`（suffix 付きなら `timeline_2.json`）を書き出す。

- [ ] **Step 1: 依存を追加する**

`crates/s2v-export/Cargo.toml` の `[dependencies]` に追加:

```toml
serde = { workspace = true }
serde_json = "1"
```

- [ ] **Step 2: 失敗するテストを書く**

`crates/s2v-export/src/exporter.rs` のテストモジュールに追加:

```rust
    #[test]
    fn timeline_json_has_relative_paths_and_names() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("audio").join("voice_0001.wav");
        let events = vec![
            TimelineEvent {
                event_type: EventType::Audio,
                start_ms: 0.0,
                duration_ms: 1500.0,
                path: Some(audio_path),
                text: Some("とうきょう".to_string()),
                display_text: Some("東京".to_string()),
                cast: Some("A".to_string()),
                name: None,
            },
            make_named_paragraph_event(1500.0, "オープニング"),
        ];
        Exporter::new(&events, dir.path(), 48000, BgmConfig::default())
            .generate_timeline_json("")
            .unwrap();

        let text = std::fs::read_to_string(dir.path().join("timeline/timeline.json")).unwrap();
        let doc: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(doc["version"], 1);
        assert_eq!(doc["sample_rate"], 48000);
        assert_eq!(doc["total_ms"], 1500.0);
        assert_eq!(doc["events"][0]["path"], "audio/voice_0001.wav");
        assert_eq!(doc["events"][0]["cast"], "A");
        assert_eq!(doc["events"][1]["event_type"], "paragraph");
        assert_eq!(doc["events"][1]["name"], "オープニング");
    }

    #[test]
    fn timeline_json_respects_suffix() {
        let dir = tempfile::tempdir().unwrap();
        let events = vec![make_paragraph_event(100.0)];
        Exporter::new(&events, dir.path(), 48000, BgmConfig::default())
            .generate_timeline_json("_2")
            .unwrap();
        assert!(dir.path().join("timeline/timeline_2.json").exists());
    }
```

- [ ] **Step 3: テストが落ちることを確認**

Run: `cargo test -p s2v-export timeline_json -- --nocapture`
Expected: FAIL（`generate_timeline_json` が存在しない）。

- [ ] **Step 4: 実装する**

`crates/s2v-export/src/exporter.rs` の `impl Exporter` 内、`generate_srt` の直後に追加:

```rust
    /// タイムラインを機械可読な JSON として書き出す（動画合成の正式入力）。
    pub fn generate_timeline_json(&self, suffix: &str) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = with_suffix(&dir.join("timeline.json"), suffix);

        let total_ms = self
            .events
            .iter()
            .map(|e| e.start_ms + e.duration_ms)
            .fold(0.0_f64, f64::max);

        let events: Vec<JsonEvent> = self
            .events
            .iter()
            .map(|e| JsonEvent {
                event_type: e.event_type.clone(),
                start_ms: e.start_ms,
                duration_ms: e.duration_ms,
                path: e.path.as_ref().map(|p| rel_path_string(p, &self.output_dir)),
                name: e.name.clone(),
                text: e.text.clone(),
                display_text: e.display_text.clone(),
                cast: e.cast.clone(),
            })
            .collect();

        let doc = serde_json::json!({
            "version": 1,
            "sample_rate": self.sample_rate,
            "total_ms": total_ms,
            "events": events,
        });
        std::fs::write(&path, serde_json::to_string_pretty(&doc)?)?;
        info!("timeline.json exported to: {}", path.display());
        Ok(())
    }
```

同ファイルのトップレベル（`impl` の外）に追加:

```rust
/// timeline.json に書き出すイベント表現。パスは project_dir 相対・スラッシュ区切りに正規化する。
#[derive(serde::Serialize)]
struct JsonEvent {
    event_type: EventType,
    start_ms: f64,
    duration_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_text: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cast: Option<String>,
}

/// project_dir 配下ならその相対パスを、外部ならフルパスを、いずれもスラッシュ区切りで返す。
fn rel_path_string(path: &Path, base: &Path) -> String {
    let p = path.strip_prefix(base).unwrap_or(path);
    p.to_string_lossy().replace('\\', "/")
}
```

`EventType` に `Clone` が無ければ `crates/s2v-core/src/timeline.rs` の derive に `Clone` を追加する（`#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]`）。

- [ ] **Step 5: テストが通ることを確認**

Run: `cargo test -p s2v-export timeline_json -- --nocapture`
Expected: PASS（2件）

- [ ] **Step 6: 本体から呼ぶ**

`src/lib.rs` の 237行付近、ロック対象ファイル一覧に `timeline.json` を追加する:

```rust
            .chain([
                self.project_root.join("timeline").join("subtitles.srt"),
                self.project_root.join("timeline").join("timeline.fcpxml"),
                self.project_root.join("timeline").join("timeline.json"),
                self.project_root.join("full_dialogue.wav"),
            ])
```

447行付近の書き出しに1行追加する:

```rust
        exporter.generate_srt(&suffix)?;
        exporter.generate_timeline_json(&suffix)?;
        exporter.generate_fcpxml(&suffix)?;
        exporter.generate_combined_audio(&suffix)?;
```

- [ ] **Step 7: 統合テストで出力を確認**

Run: `cargo test --workspace --all-targets`
Expected: PASS。`src/lib.rs` の既存統合テスト（618行付近で `subtitles.srt` を検証しているもの）に倣い、`timeline/timeline.json` が生成されることを確認するアサーションを1行足す:

```rust
        assert!(tmp.path().join("timeline").join("timeline.json").exists());
```

- [ ] **Step 8: コミット**

```bash
git add crates/s2v-export/Cargo.toml crates/s2v-export/src/exporter.rs crates/s2v-core/src/timeline.rs src/lib.rs Cargo.lock
git commit -m "feat(export): 動画合成用の timeline.json を出力する"
```

---

### Task 6: 名前付き `[PARAGRAPH]` マーカーを動画側で読めるようにする

**Files:**
- Modify: `crates/s2v-video/src/srt_timing.rs`（正規表現 6-27行、テスト）
- Modify: `crates/s2v-video/src/compose.rs`（63行付近）

**Interfaces:**
- Consumes: SRT の `[PARAGRAPH]` / `[PARAGRAPH 名称]`
- Produces: `pub struct ParagraphMarker { pub time_s: f64, pub name: Option<String> }` と `pub fn parse_paragraph_markers(srt_text: &str) -> Vec<ParagraphMarker>`。カット結合・素材解決は P1 で行うため、本タスクでは `compose` の挙動を変えない。

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-video/src/srt_timing.rs` のテストモジュールで、既存の期待値 `vec![1.5, 65.25]` を使っている2つのテスト（70行付近・126行付近）を次の形に直し、名前付きのテストを追加する:

```rust
    #[test]
    fn parses_paragraph_marker_times() {
        // 既存テストの srt リテラルはそのまま使う
        let times: Vec<f64> = parse_paragraph_markers(srt).iter().map(|m| m.time_s).collect();
        assert_eq!(times, vec![1.5, 65.25]);
    }

    #[test]
    fn parses_paragraph_marker_names() {
        let srt = "1\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH オープニング]\n\n\
                   2\n00:01:05,250 --> 00:01:05,250\n[PARAGRAPH]\n\n";
        let markers = parse_paragraph_markers(srt);
        assert_eq!(markers.len(), 2);
        assert_eq!(markers[0].time_s, 1.5);
        assert_eq!(markers[0].name.as_deref(), Some("オープニング"));
        assert_eq!(markers[1].time_s, 65.25);
        assert_eq!(markers[1].name, None);
    }
```

- [ ] **Step 2: テストが落ちることを確認**

Run: `cargo test -p s2v-video paragraph_marker -- --nocapture`
Expected: FAIL（`ParagraphMarker` が存在しない、名前付きマーカーが1件も拾えない）。

- [ ] **Step 3: 実装する**

`crates/s2v-video/src/srt_timing.rs`:

```rust
/// SRT の [PARAGRAPH] マーカー1件。
#[derive(Debug, Clone, PartialEq)]
pub struct ParagraphMarker {
    pub time_s: f64,
    pub name: Option<String>,
}

fn paragraph_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(
            r"\d+\r?\n(\d{2}):(\d{2}):(\d{2}),(\d{3}) --> \d{2}:\d{2}:\d{2},\d{3}\r?\n\[PARAGRAPH(?: ([^\]\r\n]*))?\]",
        )
        .expect("PARAGRAPH 正規表現が不正")
    })
}

/// SRTテキストから [PARAGRAPH] エントリを出現順に返す。
pub fn parse_paragraph_markers(srt_text: &str) -> Vec<ParagraphMarker> {
    let mut markers = Vec::new();
    for cap in paragraph_re().captures_iter(srt_text) {
        let h: f64 = cap[1].parse().unwrap();
        let m: f64 = cap[2].parse().unwrap();
        let s: f64 = cap[3].parse().unwrap();
        let ms: f64 = cap[4].parse().unwrap();
        let name = cap.get(5).map(|g| g.as_str().trim().to_string()).filter(|s| !s.is_empty());
        markers.push(ParagraphMarker {
            time_s: h * 3600.0 + m * 60.0 + s + ms / 1000.0,
            name,
        });
    }
    markers
}
```

- [ ] **Step 4: 呼び出し側を直す（挙動は変えない）**

`crates/s2v-video/src/compose.rs` の 63行:

```rust
    let markers: Vec<f64> = parse_paragraph_markers(&srt_text)
        .iter()
        .map(|m| m.time_s)
        .collect();
```

- [ ] **Step 5: テストが通ることを確認**

Run: `cargo test --workspace --all-targets`
Expected: PASS

- [ ] **Step 6: コミット**

```bash
git add crates/s2v-video/src/srt_timing.rs crates/s2v-video/src/compose.rs
git commit -m "feat(video): 名前付き[PARAGRAPH]マーカーを解析できるようにする"
```

---

### Task 7: ドキュメントを新仕様に合わせる

**Files:**
- Modify: `台本仕様.txt`
- Modify: `docs/manual.html`
- Modify: `claude_code_instruction.md`

**Interfaces:**
- Consumes: Task 1〜6 の確定仕様
- Produces: なし（ドキュメントのみ）

- [ ] **Step 1: 台本仕様.txt のルビ記述を直す**

@script 節の以下の記述:

```
		台詞	発声するテキスト。誤読することがあるため、誤読を防ぐため単語をシングルクォーテーションで囲み指定する。単語と読みを":"で切り分ける。
```

を次に置き換える:

```
		台詞	発声するテキスト。誤読することがあるため、誤読を防ぐため単語をシングルクォーテーションで囲み指定する。単語と読みを半角"|"で切り分ける。
			例 '東京|とうきょう' '13:00|じゅうさんじ'
			区切りは半角"|"のみ。全角"｜"は区切りとして扱わない。単語・読みのどちらにも":"を含めてよい。
```

- [ ] **Step 2: 台本仕様.txt の paragraph 記述を補う**

コマンド一覧の該当行（ユーザーが編集済み）を次に置き換える:

```
			paragraph ****	段落の区切りを示す。空白の後に名称などを記入することができる。
					名称は字幕SRTに [PARAGRAPH 名称] として出力され、動画合成のカット名・素材ファイル名として使われる。
					名称に "]" は使えない。ファイル名に使えない文字(\ / : * ? " < > |)は避けること。
```

- [ ] **Step 3: manual.html を直す**

`docs/manual.html` の 731行付近（`[PARAGRAPH]` マーカーの説明）に、名前付きマーカーの説明を1文追加する:

```html
  <code>#paragraph 名称</code> と書くと <code>[PARAGRAPH 名称]</code> として出力され、動画合成でカット名として使えます。
```

また、ルビ記法を説明している箇所があれば `|` へ差し替える（`grep -n "ルビ\|:読み" docs/manual.html` で確認する）。

- [ ] **Step 4: claude_code_instruction.md を直す**

`[PARAGRAPH]` の説明（55行・60行付近）に、名前付き形式 `[PARAGRAPH 名称]` があることを追記する。

- [ ] **Step 5: 変更を確認してコミット**

```bash
git add 台本仕様.txt docs/manual.html claude_code_instruction.md
git commit -m "docs: ルビの半角パイプ化と段落名称に合わせて仕様書・マニュアルを更新"
```

---

### Task 8: 旧台本の一括変換（使い捨てスクリプト・要ユーザー確認）

**Files:**
- Create（**スクラッチパッド**、リポジトリに入れない）: `<scratchpad>/convert_ruby_delimiter.py`
- Modify（実行結果として）: `D:\UDS\YouTube` 配下の台本 `*.txt`（約117ファイル）

**Interfaces:**
- Consumes: Task 1・2 で確定した新旧の展開ルール
- Produces: 変換済み台本と `.bak`、変換レポート（件数・要手動確認行）

- [ ] **Step 1: 変換スクリプトを書く**

`<scratchpad>/convert_ruby_delimiter.py` を作成する。要件:

```python
# 概要:
#   1. --root 配下の *.txt を再帰探索する（既定 D:\UDS\YouTube）
#   2. @script セクションの台詞行だけを対象にする
#      （@cast/@scene/@asset/@pause の定義行、# で始まる行、空行は対象外）
#   3. 旧ルビ  '([^':：]+?)[:：]([^':：]+?)'  →  '語|読み'
#   4. 役名の全角「：」→ 半角「:」（@cast に定義済みの役名で始まる行に限る）
#   5. 検証: 変換前を旧ルールで、変換後を新ルールで展開し、
#      合成テキスト・表示テキストが完全一致することをファイル単位で確認する。
#      1件でも不一致ならそのファイルは書き換えず報告する。
#   6. 既定は --dry-run。--write のときだけ .bak を作って上書きする。
#   7. 警告として報告（変換しない）:
#      - 行内のシングルクォートが奇数
#      - 全角「｜」を含む行
#      - 旧ルールで展開できないコロン入り引用
#   8. レポート: ファイル別の変換件数、要手動確認行の一覧（パス:行番号:内容）
```

旧ルール・新ルールの展開は Rust 実装と一致させる:

```python
import re
OLD_RUBY = re.compile(r"'([^':：]+?)[:：]([^':：]+?)'")
NEW_RUBY = re.compile(r"'([^'|]+?)\|([^'|]+?)'")

def expand(line, pattern):
    """(合成テキスト, 表示テキスト) を返す。Rust版 expand_ruby と同じ置換順。"""
    synthesis = display = line
    for m in pattern.finditer(line):
        full, word, reading = m.group(0), m.group(1), m.group(2)
        synthesis = synthesis.replace(full, reading)
        display = display.replace(full, word)
    return synthesis, display
```

- [ ] **Step 2: dry-run を実行してレポートを見る**

Run: `python <scratchpad>/convert_ruby_delimiter.py --root "D:\UDS\YouTube"`
Expected: 変換対象ファイル数・ルビ件数（約7,100）・全角「：」役名行（約137行）・要手動確認行の一覧が出る。検証で不一致になったファイルが0件であること。

- [ ] **Step 3: 要手動確認行を潰す**

レポートに挙がった行を1件ずつ確認し、必要なら台本を直接直す。全角「｜」を含む17行は、ルビのつもりなら半角へ、地の文なら放置でよい（判断してから次へ進む）。

- [ ] **Step 4: ユーザーに確認を取る（チェックポイント）**

117ファイルを書き換える不可逆な操作なので、レポートの要約（対象ファイル数・変換件数・手動対応した件数）を提示し、実行の可否を確認する。**承認が出るまで --write は実行しない。**

- [ ] **Step 5: 本変換を実行する**

Run: `python <scratchpad>/convert_ruby_delimiter.py --root "D:\UDS\YouTube" --write`
Expected: 各ファイルに `.bak` が作られ、変換件数がレポートされる。

- [ ] **Step 6: 抜き取りで音声生成を確認する**

直近の回を1本選んで音声生成を実行し、ルビが効いていることを確認する:

```bash
cargo run --release -- "D:\UDS\YouTube\AI道具箱\第1回.txt"
```

Expected: 未定義キャスト警告・全角「：」警告が出ないこと。`timeline/subtitles.srt` の本文が変換前と同じであること（表示テキストは語のままなので一致するはず）。

- [ ] **Step 7: 記録を残す**

Obsidian `D:\ObsidianVault\ClaudeMemory\Projects\Script2Voice-Rust版\進捗.md` に、変換日・対象ファイル数・変換件数・手動対応した行・`.bak` を残していることを追記する。スクリプト本体は保存しない。

---

### Task 9: 外部ツール・番組ドキュメントを新記法に合わせる

**Files:**
- Modify: `C:\Users\村上 孝伸\.claude\skills\ruby-annotation-verify\check_ruby.py`
- Modify: `C:\Users\村上 孝伸\.claude\skills\ruby-annotation-verify\SKILL.md`
- Modify: `D:\ObsidianVault\ClaudeMemory\Projects\YouTube共通\誤読辞書.md`
- Modify: `D:\UDS\YouTube\CLAUDE.md`、各番組の `CLAUDE.md`（`AI道具箱` / `三人寄れば・・・` / `週刊AIデスク`）

**Interfaces:**
- Consumes: 新記法 `'語|読み'`
- Produces: なし（外部ツール・ドキュメント）

- [ ] **Step 1: check_ruby.py の正規表現を差し替える**

`:` を前提にしている17箇所（70/84/87/100/103/116/126/135/146/147/177/186行付近）を `|` 前提に直す。対応表:

| 現行 | 変更後 |
|---|---|
| `r"'([ぁ-んァ-ヶー]+):([^':]+)'"` | `r"'([ぁ-んァ-ヶー]+)\|([^'\|]+)'"` |
| `r"\d+'(月\|日):[^']+'"` | `r"\d+'(月\|日)\|[^']+'"` |
| `r"'(月曜\|…)[^:]*:[^']+'"` | `r"'(月曜\|…)[^\|]*\|[^']+'"` |
| `r"'[^':]+:[^':]+'"`（ストリップ用） | `r"'[^'\|]+\|[^'\|]+'"` |
| `r"'([^':]+):[^':]+'"`（表示テキスト復元用） | `r"'([^'\|]+)\|[^'\|]+'"` |

役名行の正規表現（25行 `rf'^({cast_pattern}):(.*)$'`）と `@cast` 解析（54-55行）は半角コロンのままでよい（役名区切りは半角 `:` に確定したため）。

- [ ] **Step 2: 変換済みの実台本で check_ruby.py を回す**

Run: `python "C:\Users\村上 孝伸\.claude\skills\ruby-annotation-verify\check_ruby.py" "D:\UDS\YouTube\AI道具箱\第1回.txt"`
Expected: 変換前に実行したときと同じ指摘内容になる（新記法を認識できている証拠）。差分が出たら正規表現の修正漏れ。

- [ ] **Step 3: SKILL.md の記法説明を直す**

`## Ruby Syntax` 節の `'漢字:よみ'` を `'漢字|よみ'` に、frontmatter の `description` にある `'kanji:yomi'` を `'kanji|yomi'` に、Quick Reference 表の例（`'それ:それ'`、`'は:わ'`、`8'月:がつ'` 等）をすべて `|` に直す。

- [ ] **Step 4: 誤読辞書・番組 CLAUDE.md を直す**

Run: `grep -rn "':" "D:\ObsidianVault\ClaudeMemory\Projects\YouTube共通\誤読辞書.md" "D:\UDS\YouTube\CLAUDE.md" "D:\UDS\YouTube\AI道具箱\CLAUDE.md" "D:\UDS\YouTube\三人寄れば・・・\CLAUDE.md"`
で記法例を洗い出し、`|` に差し替える。

- [ ] **Step 5: 動作確認**

変換済み台本1本に対して `check_ruby.py` が `RESULT: PASS`（または想定内ノイズのみ）になることを確認する。

- [ ] **Step 6: リポジトリ側の差分をコミット**

外部ファイル（スキル・Obsidian・番組フォルダ）は git 管理外なのでコミット対象外。リポジトリ内に変更があれば:

```bash
git status
git commit -am "docs: 外部ツールの記法更新に伴う追随（あれば）"
```

---

## 後続フェーズ（本計画の範囲外）

P1（カット層）・P2（キャラクター層）・P3（口パク）・P4（表情指定）は、それぞれ着手時に個別の実装計画を書く。P0 完了時点で成立している前提は次の3つ:

- `timeline/timeline.json` が出ている（P1・P2 の入力）
- SRT に `[PARAGRAPH 名称]` が出ている（P1 の入力、旧プロジェクト用フォールバック）
- `parse_paragraph_markers` が名前を返す（P1 のカット導出が使う）

---

## Self-Review

**1. Spec coverage（P0 該当分）**

| 仕様書の項目 | 対応タスク |
|---|---|
| 1.1 ルビ半角パイプ化・旧記法廃止・全角｜警告 | Task 1 |
| 1.2 役名区切り半角化・警告改善 | Task 2 |
| 1.3 `#paragraph 名称`・禁止文字の警告 | Task 3 |
| 1.4 台本仕様.txt の更新 | Task 2 Step 6 / Task 7 |
| 2.1 SRT `[PARAGRAPH 名称]` | Task 4 |
| 2.2 `timeline.json` | Task 5 |
| 5章 後方互換（無名台本は従来と同一） | Task 4 Step 7 / Task 6 Step 4（挙動不変を明示） |
| 9章 ドキュメント・外部ツール更新 | Task 7 / Task 9 |
| 10章 旧台本の一括変換（使い捨て・.bak必須・記録） | Task 8 |

P0 に含めないもの（仕様書 2.3 / 2.4 / 3章 / 4章）は「後続フェーズ」に明記済み。

**2. Placeholder scan**: 各ステップに実コードまたは実行コマンドを記載済み。Task 8 のスクリプトは要件コメント＋一致必須の展開関数を提示しており、実装者が判断で埋める余地は「レポート整形」のみ。

**3. Type consistency**: `ScriptCommand::Paragraph(Option<String>)`（Task 3）→ `register_paragraph(Option<String>)`（Task 4）→ `TimelineEvent.name`（Task 4）→ `JsonEvent.name`（Task 5）、`ParagraphMarker { time_s, name }`（Task 6）で一貫。Task 3 Step 5 の暫定実装（`_name` で受けて捨てる）→ Task 4 Step 6 で本結線、という順序依存も明記済み。
