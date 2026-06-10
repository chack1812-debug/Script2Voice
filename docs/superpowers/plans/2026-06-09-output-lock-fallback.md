# 出力ロック時の生成全体・共通連番フォールバック 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 出力ファイルのいずれかが使用中(ロック)のとき、その生成の全成果物(個別音声＋SRT＋FCPXML＋統合音声)を共通の連番サフィックス `_N` で保存し、エラー終了せず合成を無駄にしない。

**Architecture:** s2v-export に純粋関数 `with_suffix`/`is_path_writable`/`resolve_generation_suffix` を追加。`src/lib.rs` が Phase2 の前に既定名ファイル一式を probe して世代サフィックスを1回決定し、各タスクの `final_path` とエクスポート出力名に適用する。FCPXML は events のパス(=連番音声)をそのまま参照するため自動整合。

**Tech Stack:** Rust / s2v-export / s2v-core / 本体 — 新規依存なし

設計書: `docs/superpowers/specs/2026-06-09-output-lock-fallback-design.md`

**コンパイル順:** `generate_*` のシグネチャ変更は `src/lib.rs` の呼び出しを壊すため、Task2 で同時に更新する(lib.rs は当面 `""` を渡す)。Task3 で実際の世代サフィックス決定を配線する。Task1 は加算的で単独緑。

---

## Task 1: s2v-export に連番ユーティリティを追加

**Files:**
- Modify: `crates/s2v-export/src/exporter.rs`（自由関数3つ＋テスト）
- Modify: `crates/s2v-export/src/lib.rs`（re-export）

- [ ] **Step 1: 失敗テストを書く** — `crates/s2v-export/src/exporter.rs` の `mod tests` 末尾に追加:
```rust
    #[test]
    fn with_suffix_inserts_before_extension() {
        assert_eq!(with_suffix(Path::new("a/voice_0001.wav"), "_3"), PathBuf::from("a/voice_0001_3.wav"));
        assert_eq!(with_suffix(Path::new("subtitles.srt"), "_2"), PathBuf::from("subtitles_2.srt"));
        assert_eq!(with_suffix(Path::new("noext"), "_1"), PathBuf::from("noext_1"));
        assert_eq!(with_suffix(Path::new("x.wav"), ""), PathBuf::from("x.wav"));
    }

    #[test]
    fn is_path_writable_true_for_missing_and_normal_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(is_path_writable(&dir.path().join("nope.wav")));
        let f = dir.path().join("ok.txt");
        std::fs::write(&f, b"x").unwrap();
        assert!(is_path_writable(&f));
    }

    #[test]
    fn is_path_writable_false_for_directory_named_like_file() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("locked.wav");
        std::fs::create_dir(&d).unwrap();
        assert!(!is_path_writable(&d));
    }

    #[test]
    fn resolve_suffix_empty_when_all_writable() {
        let dir = tempfile::tempdir().unwrap();
        let files = vec![dir.path().join("a.wav"), dir.path().join("b.srt")];
        assert_eq!(resolve_generation_suffix(&files, 100).unwrap(), "");
    }

    #[test]
    fn resolve_suffix_falls_back_when_one_locked() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        std::fs::create_dir(&a).unwrap(); // a.wav をディレクトリにして書込不可(=ロック相当)
        let b = dir.path().join("b.srt");
        let files = vec![a, b];
        assert_eq!(resolve_generation_suffix(&files, 100).unwrap(), "_1");
    }

    #[test]
    fn resolve_suffix_skips_existing_numbered_slot() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a.wav");
        std::fs::create_dir(&a).unwrap();
        let b = dir.path().join("b.srt");
        std::fs::write(&b, b"x").unwrap();
        std::fs::write(dir.path().join("a_1.wav"), b"x").unwrap(); // _1 スロットを一部埋める
        let files = vec![a, b];
        assert_eq!(resolve_generation_suffix(&files, 100).unwrap(), "_2");
    }
```

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-export with_suffix_inserts_before_extension` → FAIL（関数未定義でコンパイルエラー）。

- [ ] **Step 3: 自由関数を実装** — `crates/s2v-export/src/exporter.rs` の、`impl<'a> Exporter<'a>` ブロックの**後**（ファイル下部、`mod tests` の前あたり）に追加:
```rust
/// ファイル名の拡張子の前に suffix を挿入する。suffix が空ならパスをそのまま返す。
/// 例: with_suffix("voice_0001.wav", "_3") == "voice_0001_3.wav"
pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    if suffix.is_empty() {
        return path.to_path_buf();
    }
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let name = match path.extension() {
        Some(ext) => format!("{stem}{suffix}.{}", ext.to_string_lossy()),
        None => format!("{stem}{suffix}"),
    };
    match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.join(name),
        _ => PathBuf::from(name),
    }
}

/// パスが書き込み可能か（=使用中でないか）。非存在は true。
/// 既存ファイルは truncate せずに書き込みオープンを試し、成否で判定する。
pub fn is_path_writable(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

/// 生成の既定名ファイル一式から世代サフィックスを決める。
/// すべて書込可なら ""。いずれか使用中なら、一式の `_n` 版がすべて未存在になる最小の `_n`。
pub fn resolve_generation_suffix(default_files: &[PathBuf], max: usize) -> anyhow::Result<String> {
    let needs_fallback = default_files.iter().any(|p| p.exists() && !is_path_writable(p));
    if !needs_fallback {
        return Ok(String::new());
    }
    for n in 1..=max {
        let suffix = format!("_{n}");
        if default_files.iter().all(|p| !with_suffix(p, &suffix).exists()) {
            return Ok(suffix);
        }
    }
    anyhow::bail!("使用中の出力を回避する空き連番({max}まで)が見つかりませんでした")
}
```

- [ ] **Step 4: re-export** — `crates/s2v-export/src/lib.rs` を:
```rust
pub mod exporter;

pub use exporter::{is_path_writable, resolve_generation_suffix, with_suffix, Exporter};
```

- [ ] **Step 5: テスト通過確認** — Run: `cargo test -p s2v-export` → PASS（新6テスト＋既存全通過）。

- [ ] **Step 6: コミット**
```bash
git add crates/s2v-export/src/exporter.rs crates/s2v-export/src/lib.rs
git commit -m "feat(export): add with_suffix/is_path_writable/resolve_generation_suffix utilities"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 2: generate_* をサフィックス引数対応に（lib.rs は当面 "" を渡す）

**Files:**
- Modify: `crates/s2v-export/src/exporter.rs`（3メソッドのシグネチャ＋出力名、既存テスト更新、新テスト）
- Modify: `src/lib.rs`（呼び出しに `""` を渡す）

- [ ] **Step 1: 新テストを書く** — `crates/s2v-export/src/exporter.rs` の `mod tests` 末尾に追加（既存ヘルパー `write_wav`(line 512付近)・`make_audio_event`(line 484付近)・`default_bgm` を使用）:
```rust
    #[test]
    fn generate_outputs_with_suffix_writes_numbered_names() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path();
        let wav = out_dir.join("voice_0001_2.wav");
        write_wav(&wav, 48000, 0.1);
        let events = vec![make_audio_event(0.0, 100.0, "テスト", Some(wav.clone()))];
        let exp = Exporter::new(&events, out_dir, 48000, default_bgm());
        exp.generate_srt("_2").unwrap();
        exp.generate_fcpxml("_2").unwrap();
        exp.generate_combined_audio("_2").unwrap();
        assert!(out_dir.join("timeline/subtitles_2.srt").exists());
        assert!(out_dir.join("timeline/timeline_2.fcpxml").exists());
        assert!(out_dir.join("full_dialogue_2.wav").exists());
    }

    #[test]
    fn fcpxml_references_suffixed_voice_path() {
        let dir = tempfile::tempdir().unwrap();
        let out_dir = dir.path();
        let wav = out_dir.join("voice_0001_2.wav");
        write_wav(&wav, 48000, 0.1);
        let events = vec![make_audio_event(0.0, 100.0, "テスト", Some(wav.clone()))];
        let exp = Exporter::new(&events, out_dir, 48000, default_bgm());
        exp.generate_fcpxml("_2").unwrap();
        let xml = std::fs::read_to_string(out_dir.join("timeline/timeline_2.fcpxml")).unwrap();
        assert!(xml.contains("voice_0001_2.wav"), "FCPXMLは連番付き音声を参照すること: {xml}");
    }
```
（`default_bgm` ヘルパーが無い場合は、既存テストが `Exporter::new` に渡している BgmConfig 生成方法に合わせること。例えば既存テストで使われている生成式をそのまま使う。）

- [ ] **Step 2: 失敗確認** — Run: `cargo test -p s2v-export generate_outputs_with_suffix_writes_numbered_names` → FAIL（`generate_srt` が引数を取らないためコンパイルエラー）。

- [ ] **Step 3: 3メソッドをサフィックス対応に** — `crates/s2v-export/src/exporter.rs`:
`generate_srt`:
```rust
    pub fn generate_srt(&self, suffix: &str) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = with_suffix(&dir.join("subtitles.srt"), suffix);
```
（以降の `path` 使用箇所は不変）

`generate_fcpxml`:
```rust
    pub fn generate_fcpxml(&self, suffix: &str) -> anyhow::Result<()> {
        let dir = self.output_dir.join("timeline");
        std::fs::create_dir_all(&dir)?;
        let path = with_suffix(&dir.join("timeline.fcpxml"), suffix);
```
（現在 `let path = dir.join("timeline.fcpxml");` を上記2行に置換。以降不変）

`generate_combined_audio`:
```rust
    pub fn generate_combined_audio(&self, suffix: &str) -> anyhow::Result<()> {
        let out_path = with_suffix(&self.output_dir.join("full_dialogue.wav"), suffix);
```
（現在 `let out_path = self.output_dir.join("full_dialogue.wav");` を置換。以降不変。スキップ時の早期 `Ok` は維持）

- [ ] **Step 4: 既存 exporter テストの generate_* 呼び出しを更新** — `crates/s2v-export/src/exporter.rs` の `mod tests` 内、`generate_srt()` / `generate_fcpxml()` / `generate_combined_audio()` を呼んでいる**全箇所**に引数 `""` を渡す（例 `exp.generate_combined_audio("").unwrap();`）。`grep -n 'generate_srt(\|generate_fcpxml(\|generate_combined_audio(' crates/s2v-export/src/exporter.rs` で該当を洗い出し漏れなく更新すること（Step1 で追加した新テストは既に引数付きなので対象外）。

- [ ] **Step 5: lib.rs の呼び出しに "" を渡す（暫定）** — `src/lib.rs` のエクスポート呼び出し3行を:
```rust
        exporter.generate_srt("")?;
        exporter.generate_fcpxml("")?;
        exporter.generate_combined_audio("")?;
```
（Task3 で実際の `&suffix` に置き換える）

- [ ] **Step 6: テスト通過確認** — Run: `cargo test --workspace` → PASS（新2テスト＋既存全通過。`""` 指定で従来と同じ既定名）。

- [ ] **Step 7: コミット**
```bash
git add crates/s2v-export/src/exporter.rs src/lib.rs
git commit -m "feat(export): thread output suffix through generate_srt/fcpxml/combined"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 3: lib.rs で世代サフィックスを決定・配線

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Phase2 の前にサフィックスを決定し final_path に適用** — `src/lib.rs` の `produce` で、`info!("Phase1完了: ...")` の**直後**（プリウォームの前）に追加:
```rust
        // ── 出力ロック対策: 生成一式の共通連番サフィックスを決定 ───────────
        let default_files: Vec<PathBuf> = tasks.iter()
            .map(|(_, _, t)| t.final_path.clone())
            .chain([
                self.project_root.join("timeline").join("subtitles.srt"),
                self.project_root.join("timeline").join("timeline.fcpxml"),
                self.project_root.join("full_dialogue.wav"),
            ])
            .collect();
        let suffix = s2v_export::resolve_generation_suffix(&default_files, 100)?;
        if !suffix.is_empty() {
            warn!("出力ファイルのいずれかが使用中のため、今回の生成一式を連番 {suffix} で保存します。");
        }
        for (_, _, t) in tasks.iter_mut() {
            t.final_path = s2v_export::with_suffix(&t.final_path, &suffix);
        }
```
（`tasks` は `let mut tasks` で宣言済み。`PathBuf`・`warn` は import 済み。`s2v_export` は依存に含まれる。）

- [ ] **Step 2: エクスポート呼び出しを suffix に切り替え** — `src/lib.rs` の Task2 Step5 で `""` にした3行を:
```rust
        exporter.generate_srt(&suffix)?;
        exporter.generate_fcpxml(&suffix)?;
        exporter.generate_combined_audio(&suffix)?;
```

- [ ] **Step 3: ビルドと全テスト** — Run: `cargo build && cargo test --workspace` → PASS（全クレートがビルドでき全テスト通過。ロックが無い通常時は suffix="" で従来どおり）。

- [ ] **Step 4: スモーク（正常時の回帰）** — Run: `cargo run -- <任意の台本.txt>`（config.toml を実行ファイル隣に置く前提）。エラーなく完了し、既定名（`full_dialogue.wav`/`timeline/subtitles.srt`/`timeline/timeline.fcpxml`/`audio/voice_*.wav`）で出力されることを確認。

- [ ] **Step 5: コミット**
```bash
git add src/lib.rs
git commit -m "feat: resolve generation suffix before synthesis to survive locked outputs"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Task 4: ドキュメント（トラブルシューティング）と最終確認

**Files:**
- Modify: `docs/manual.html`

- [ ] **Step 1: manual.html にトラブルシューティング項を追記** — `docs/manual.html` のトラブルシューティング相当のセクションに、次の趣旨の短い項目を追加する（既存の見出し/書式に合わせる。Edit ツールで UTF-8 を保つ）:
  - 「出力ファイル（`full_dialogue.wav` / `timeline/*.srt` / `*.fcpxml` / `audio/voice_*.wav`）を動画編集ソフト等で**開いたまま**再実行すると、ファイルが使用中で書き込めない。その場合プログラムは異常終了せず、**今回の生成一式すべてを連番 `_N`（例 `full_dialogue_1.wav`, `voice_0001_1.wav`, `timeline_1.fcpxml` …）で保存**し、警告ログを出す。FCPXML は同じ `_N` の音声を参照するので整合は保たれる。前世代の既定名ファイルはそのまま残る。」
  該当セクションが無ければ、適切な見出しの近くに新しい小見出しで追加する。

- [ ] **Step 2: 全体テスト・リリース** — Run: `cargo test --workspace` → 全 PASS（件数記録）。`cargo build --release` → 成功確認。

- [ ] **Step 3: コミット**
```bash
git add docs/manual.html
git commit -m "docs: document output-lock numbered fallback behavior"
```
末尾に `Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>`。

---

## Self-Review メモ

- **Spec coverage**: §1 with_suffix/適用範囲=Task1(with_suffix)+Task2(generate_*)+Task3(final_path)。§2 決定/probe=Task1(is_path_writable/resolve_generation_suffix)+Task3(配線)。§3 配線=Task3。§4 エクスポータ変更=Task2。§5 テスト=Task1/2。ドキュメント=Task4。
- **Placeholder scan**: なし（Task2 の `default_bgm` は「無ければ既存テストの BgmConfig 生成に合わせる」と明記）。
- **Type consistency**: `with_suffix(&Path, &str)->PathBuf` / `is_path_writable(&Path)->bool` / `resolve_generation_suffix(&[PathBuf], usize)->Result<String>`（Task1）、`generate_srt/fcpxml/combined(&self, suffix:&str)`（Task2）、lib.rs の `resolve_generation_suffix(&default_files,100)` と `with_suffix(&t.final_path,&suffix)` と `generate_*(&suffix)`（Task3）は定義と使用で一致。
- **コンパイル順**: generate_* シグネチャ変更(Task2)は lib.rs 呼び出しを壊すため同タスクで `""` を渡して緑化。Task3 で実サフィックスに差し替え。Task1 は加算的。
