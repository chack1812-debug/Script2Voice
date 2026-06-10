# SRT [PARAGRAPH] マーカー出力 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 台本中の `#paragraph` コマンドが出現した時刻を、Rust版Script2Voiceが出力する `subtitles.srt` に `[PARAGRAPH]` というゼロ秒字幕エントリとして埋め込む（後段の動画合成パイプラインがシーン切り替えタイミングを検出できるようにするため）。

**Architecture:** タイムライン構築時 (`#paragraph` コマンド処理時) に、ポーズを加算する直前の `current_ms`（＝直前のセリフの終了時刻）で新しい `EventType::Paragraph` イベントを登録する。SRTエクスポータは `Audio` イベントと `Paragraph` イベントの両方を時刻順にマージして連番を振り、`Paragraph` は `start == end`（ゼロ秒）・テキスト固定 `[PARAGRAPH]` で出力する。

**Tech Stack:** Rust (workspace: s2v-core, s2v-export, ルートクレート), cargo test

---

## File Structure

- Modify: `crates/s2v-core/src/timeline.rs`
  - `EventType` に `Paragraph` バリアントを追加
  - `TimelineProcessor::register_paragraph()` を新設（`current_ms` でゼロ秒の `Paragraph` イベントを登録）
- Modify: `src/lib.rs`
  - `ScriptCommand::Paragraph` の処理を「`register_paragraph()` を呼んでから `advance_paragraph()` する」順序に変更
- Modify: `crates/s2v-export/src/exporter.rs`
  - `generate_srt()` を `Audio` と `Paragraph` の混在マージ＋連番採番に書き換え
  - テスト用ヘルパー `make_paragraph_event` を追加し、混在ケースのテストを追加

No new files needed; all changes land in existing, focused modules that already own this responsibility.

---

## Task 1: `EventType::Paragraph` と `register_paragraph` を追加する

**Files:**
- Modify: `crates/s2v-core/src/timeline.rs:9-14` (enum), `:120-127` (impl の末尾, `register_se` の直後)
- Test: `crates/s2v-core/src/timeline.rs` 内 `mod tests`（`register_bgm_uses_current_time` の付近に追加）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-core/src/timeline.rs` の `mod tests` 内、`register_bgm_uses_current_time` テストの直後に追加:

```rust
    #[test]
    fn register_paragraph_uses_current_time_and_zero_duration() {
        let mut tp = TimelineProcessor::new(&default_pause());
        tp.advance_after_speech(1000.0, None);
        let before = tp.current_ms;
        tp.register_paragraph();
        let events = tp.get_events();
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_type, EventType::Paragraph);
        assert!((events[0].start_ms - before).abs() < 1e-10);
        assert!((events[0].duration_ms - 0.0).abs() < 1e-10);
        assert_eq!(events[0].display_text.as_deref(), Some("[PARAGRAPH]"));
        // current_ms は変化しない (advance は呼び出し側が別途行う)
        assert!((tp.current_ms - before).abs() < 1e-10);
    }
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-core register_paragraph_uses_current_time_and_zero_duration -- --nocapture`
Expected: FAIL — `EventType::Paragraph` と `register_paragraph` が存在せずコンパイルエラーになる（`no variant or associated item named 'Paragraph' found` / `no method named 'register_paragraph' found`）

- [ ] **Step 3: 最小実装を書く**

`crates/s2v-core/src/timeline.rs:9-14` の `EventType` enum を以下に変更:

```rust
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    Audio,
    BgmStart,
    BgmStop,
    Se,
    Paragraph,
}
```

`crates/s2v-core/src/timeline.rs:109-119` の `register_se` の直後（`impl TimelineProcessor` 内、`get_events` の手前）に以下を追加:

```rust
    pub fn register_paragraph(&mut self) {
        self.events.push(TimelineEvent {
            event_type: EventType::Paragraph,
            start_ms: self.current_ms,
            duration_ms: 0.0,
            path: None,
            text: None,
            display_text: Some("[PARAGRAPH]".to_string()),
            cast: None,
        });
    }
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cargo test -p s2v-core register_paragraph_uses_current_time_and_zero_duration -- --nocapture`
Expected: PASS（1 passed; 0 failed）

念のため timeline.rs の全テストも実行する:
Run: `cargo test -p s2v-core --lib timeline::`
Expected: 全件 PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-core/src/timeline.rs
git commit -m "feat(core): add EventType::Paragraph and register_paragraph for [PARAGRAPH] markers"
```

---

## Task 2: `#paragraph` コマンド処理でイベントを登録する

**Files:**
- Modify: `src/lib.rs:256`
- Test: `tests/e2e.rs`（既存のe2eテストの構造を確認した上で、`#paragraph` を含む台本を実行し `subtitles.srt` に `[PARAGRAPH]` 行が含まれることを検証するテストを追加）

- [ ] **Step 1: e2eテスト用の台本に `#paragraph` を仕込み、出力を検証するアサーションを追加する**

まず既存の `tests/e2e.rs` を読み、台本フィクスチャ（`@script` ブロックや一時ディレクトリへの書き出し方法）と `subtitles.srt` を読み込んでいる箇所を特定する。既存テストが `#paragraph` を含む台本を使っていない場合は、台本フィクスチャ文字列に下記のような1行を追加する:

```text
#paragraph
```

（セリフ行の直後、次のセリフの前に挿入。挿入位置は既存フィクスチャの構造に合わせて調整する。）

そのうえで、SRT読み込み後のアサーション群に以下を追加する:

```rust
    assert!(srt_content.contains("[PARAGRAPH]"));
```

具体的な変数名（`srt_content` 等）は既存テストの命名に合わせること。

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test --test e2e -- --nocapture`
Expected: FAIL — `assertion failed: srt_content.contains("[PARAGRAPH]")`（まだ `[PARAGRAPH]` が出力されていないため）

- [ ] **Step 3: 最小実装を書く**

`src/lib.rs:256` の現在のコード:

```rust
                            ScriptCommand::Paragraph => timeline.advance_paragraph(),
```

を以下に変更（イベント登録 → ポーズ加算の順序。タイムスタンプは「`#paragraph` 直前のセリフの終了時刻」、すなわちポーズを加算する前の `current_ms` を使う必要があるため、この順序が必須）:

```rust
                            ScriptCommand::Paragraph => {
                                timeline.register_paragraph();
                                timeline.advance_paragraph();
                            }
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cargo test --test e2e -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add src/lib.rs tests/e2e.rs
git commit -m "feat: register [PARAGRAPH] timeline event at #paragraph command position"
```

---

## Task 3: SRTエクスポータで `Audio` と `Paragraph` をマージ出力する

**Files:**
- Modify: `crates/s2v-export/src/exporter.rs:28-53` (`generate_srt`)
- Test: `crates/s2v-export/src/exporter.rs` 内 `mod tests`（`make_audio_event` の直後に `make_paragraph_event` を追加し、`srt_generates_correct_format` の直後に混在ケースのテストを追加）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-export/src/exporter.rs` の `mod tests` 内、`make_audio_event` 関数 (line 479-489) の直後に以下のヘルパーを追加:

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
        }
    }
```

`srt_generates_correct_format` テスト (line 510-527) の直後に以下のテストを追加:

```rust
    #[test]
    fn srt_includes_paragraph_markers_in_chronological_order_with_continuous_numbering() {
        let events = vec![
            make_audio_event(0.0, 1500.0, "こんにちは", None),
            make_paragraph_event(1500.0),
            make_audio_event(3000.0, 800.0, "さようなら", None),
        ];
        let dir = tempfile::tempdir().unwrap();
        let exp = Exporter::new(&events, dir.path(), 48000, default_bgm());
        exp.generate_srt().unwrap();

        let content = std::fs::read_to_string(dir.path().join("timeline/subtitles.srt")).unwrap();
        // 1: 通常の字幕
        assert!(content.contains("1\n00:00:00,000 --> 00:00:01,500\nこんにちは\n"));
        // 2: ゼロ秒の [PARAGRAPH] エントリ。タイムスタンプは直前のセリフの終了時刻と同一
        assert!(content.contains("2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n"));
        // 3: 通常の字幕（連番が続く）
        assert!(content.contains("3\n00:00:03,000 --> 00:00:03,800\nさようなら\n"));
    }
```

- [ ] **Step 2: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-export srt_includes_paragraph_markers -- --nocapture`
Expected: FAIL — 現状の `generate_srt` は `EventType::Audio` のみをフィルタするため、`[PARAGRAPH]` 行が出力されず `2\n00:00:01,500 --> 00:00:01,500\n[PARAGRAPH]\n` を含むアサーションが失敗する

- [ ] **Step 3: 最小実装を書く**

`crates/s2v-export/src/exporter.rs:28-53` の `generate_srt` を以下に置き換える:

```rust
    pub fn generate_srt(&self) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("subtitles.srt");

        let mut subtitle_events: Vec<_> = self.events.iter()
            .filter(|e| e.event_type == EventType::Audio || e.event_type == EventType::Paragraph)
            .collect();
        subtitle_events.sort_by(|a, b| a.start_ms.partial_cmp(&b.start_ms).unwrap());

        let mut content = String::new();
        for (i, event) in subtitle_events.iter().enumerate() {
            let start_s = event.start_ms / 1000.0;
            let end_s = match event.event_type {
                EventType::Paragraph => start_s,
                _ => (event.start_ms + event.duration_ms) / 1000.0,
            };
            content.push_str(&format!(
                "{}\n{} --> {}\n{}\n\n",
                i + 1,
                format_srt_time(start_s),
                format_srt_time(end_s),
                event.display_text.as_deref().unwrap_or(""),
            ));
        }

        std::fs::write(&path, &content)?;
        info!("SRT exported to: {}", path.display());
        Ok(())
    }
```

- [ ] **Step 4: テストを実行して成功を確認する**

Run: `cargo test -p s2v-export srt_includes_paragraph_markers -- --nocapture`
Expected: PASS

既存の `srt_generates_correct_format` も壊れていないことを確認する:
Run: `cargo test -p s2v-export srt_generates_correct_format -- --nocapture`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-export/src/exporter.rs
git commit -m "feat(export): merge Paragraph events into SRT as zero-length [PARAGRAPH] markers"
```

---

## Task 4: ワークスペース全体のテストを流す

**Files:** なし（検証のみ）

- [ ] **Step 1: ワークスペース全体のテストを実行する**

Run: `cargo test --workspace`
Expected: 全件 PASS（既存テストを含め回帰がないこと）

- [ ] **Step 2: 問題なければ完了。問題があれば該当タスクに戻って修正し、再実行する**

---

## Self-Review チェックリスト（実行前に1回だけ目視確認）

- 仕様カバレッジ: 「タイムスタンプは`#paragraph`直前のセリフの終了時刻と同じ値を使う（ゼロ秒エントリ）」→ Task 1-3で対応。「連番Nは通常の字幕と連続した番号にする」→ Task 3 のマージ＋`enumerate`で対応。「テキストは`[PARAGRAPH]`固定」→ Task 1, 3 で対応。
- プレースホルダ: なし（すべてのコードブロックは完全な差分）。
- 型の一貫性: `EventType::Paragraph` / `register_paragraph` / `make_paragraph_event` の名前は全タスクで統一。
