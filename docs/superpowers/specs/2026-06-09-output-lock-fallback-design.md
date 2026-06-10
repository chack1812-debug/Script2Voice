# 出力ロック時の生成全体・共通連番フォールバック 設計書

作成日: 2026-06-09

## 背景・目的

生成は Phase1(タスク割当) → Phase2(合成・音響処理で `voice_NNNN.wav` を固定名で書き出し) →
Phase3(タイムライン構築) → エクスポート(`subtitles.srt` / `timeline.fcpxml` / `full_dialogue.wav`) の順。
出力ファイルのいずれかが他アプリで開かれて**使用中(ロック)**だと、書き込みがエラーになり `?` で伝播して
**異常終了**し、TTS 合成にかけた時間が無駄になる。

特に、**前の生成を動画編集ソフトで開いたまま再実行**すると、ソフトは FCPXML が参照する個別音声
(`voice_*.wav`) も掴むため、Phase2 の上書きで失敗する。FCPXML は個別音声を絶対パスで参照するので、
出力だけ連番化しても音声との整合が崩れる懸念がある。

本機能は、**いずれかの出力が使用中のとき、この生成の全成果物（個別音声＋SRT＋FCPXML＋統合音声）を
共通の連番サフィックス `_N` を付けて保存**する。これにより、前世代を編集中でも合成を無駄にせず、
`_N` 一式が互いに整合した状態で残る。FCPXML は同じ `_N` の音声を参照する。

## スコープ

- **やること**
  - 生成1回ぶんの全成果物に**共通の世代サフィックス**（`""` または `"_N"`）を付ける仕組み。
  - Phase2 の前に、既定名の生成ファイル一式の書き込み可否を probe してサフィックスを1回決定する。
  - サフィックスを個別音声の `final_path` と SRT/FCPXML/統合音声のファイル名に適用する。
  - フォールバック時は WARNING ログを出し、`Ok` を返して終了しない。
- **やらないこと**
  - 中間一時ファイル `voice_NNNN_raw.wav`（合成直後、FCPXML 非参照）への適用（既定名のまま）。
  - ロック以外の書き込み失敗（ディスク満杯等）の特別扱い（通常どおりエラー）。
  - Python 版への移植。

## 1. 世代サフィックスと適用範囲

1回の生成の「成果物セット」（既定名、`project_root` 基準）:
- 個別音声: 各タスクの `final_path` = `audio/voice_NNNN.wav`。
- 字幕: `timeline/subtitles.srt`。
- タイムライン: `timeline/timeline.fcpxml`。
- 統合音声: `full_dialogue.wav`（ミックス対象が無ければ生成スキップ。probe 対象には含めるが、存在しなければ「書込可」扱い）。

世代サフィックス `suffix`（`""` か `"_N"`）を、上記すべての**ファイル名の拡張子の前**に挿入する:
- `voice_0001_3.wav` / `subtitles_3.srt` / `timeline_3.fcpxml` / `full_dialogue_3.wav`（例 N=3）。

純粋関数 `with_suffix(path: &Path, suffix: &str) -> PathBuf`:
- 拡張子があれば `stem + suffix + "." + ext`、無ければ `file_name + suffix`。親ディレクトリは不変。
- 例: `with_suffix("audio/voice_0001.wav", "_3") == "audio/voice_0001_3.wav"`、`with_suffix(p, "") == p`。

## 2. サフィックスの決定（Phase1 の後・Phase2 の前）

`src/lib.rs` の `produce` で、Phase1 完了直後（プリウォーム・Phase2 の前）に1回だけ実行する。

入力 = この生成の既定名ファイル一式 `default_files`（全タスクの `final_path` ＋ `timeline/subtitles.srt`
＋ `timeline/timeline.fcpxml` ＋ `full_dialogue.wav`）。

決定 `resolve_generation_suffix(default_files: &[PathBuf], max: usize) -> anyhow::Result<String>`（s2v-export 提供）:
1. `default_files` のうち**存在する**もので、1つでも書き込み不可（ロック）なものがあれば「フォールバック必要」。
   無ければ `Ok("".into())`。
2. フォールバック時、`n = 1..=max(=100)` の順に、`default_files` 全要素の `with_suffix(p, "_n")` が
   **いずれも未存在**になる最小の `n` を探し、`Ok(format!("_{n}"))`。
3. 見つからなければ `Err`（異常事態）。

書き込み可否 `is_path_writable(path: &Path) -> bool`（s2v-export 提供）:
- 非存在 → `true`（これから作るだけ）。
- 存在 → `OpenOptions::new().write(true).open(path)`（**truncate しない**）が `Ok` なら `true`、`Err` なら `false`。
- truncate しないので probe で既存ファイルを破壊しない。

## 3. 配線（src/lib.rs）

```
// Phase1 完了後
let default_files: Vec<PathBuf> = tasks.iter().map(|(_,_,t)| t.final_path.clone())
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
// 各タスクの final_path にサフィックスを適用（raw_path は既定名のまま）
for (_, _, t) in tasks.iter_mut() {
    t.final_path = s2v_export::with_suffix(&t.final_path, &suffix);
}
```
- Phase2（合成）は suffix 付き `final_path` に書く。Phase3 の `register_audio(t.final_path)` 経由で
  TimelineEvent.path も suffix 付き → FCPXML は `voice_*_N.wav` を参照（自動整合）。
- エクスポート呼び出しに suffix を渡す:
  `exporter.generate_srt(&suffix)?; exporter.generate_fcpxml(&suffix)?; exporter.generate_combined_audio(&suffix)?;`
  - 各メソッドは `with_suffix` で出力ファイル名を組み立てる（**再 probe はしない**。決定は §2 で済み）。
- 既定名の旧世代ファイル（編集ソフトが開いているもの）は**削除しない**（温存）。

注: §2 の probe（Phase2 前）と実際の書き込み（Phase2/エクスポート）の間に新たにロックが発生する TOCTOU は
まれに起こり得るが、その場合は従来どおり `?` でエラーになる（持続的に開いている主要ケースは probe で検知できる）。

## 4. エクスポータ変更（crates/s2v-export/src/exporter.rs）

- `pub fn with_suffix(path: &Path, suffix: &str) -> PathBuf` と `pub fn is_path_writable(path: &Path) -> bool`、
  `pub fn resolve_generation_suffix(default_files: &[PathBuf], max: usize) -> anyhow::Result<String>` を追加（自由関数）。
- `generate_srt(&self, suffix: &str)` / `generate_fcpxml(&self, suffix: &str)` /
  `generate_combined_audio(&self, suffix: &str)` に変更。各々 `with_suffix(基準パス, suffix)` で書き出す。
- `generate_combined_audio` の既存スキップ（ミックス対象なし→早期 `Ok`）は維持。
- FCPXML の参照（`build_resource_tags`）は `events[].path`（= suffix 付き音声パス）をそのまま使うため変更不要。

## 5. テスト

s2v-export:
- `with_suffix`: 拡張子あり/なし、`""`。
- `is_path_writable`: 非存在→true、書ける既存→true、ディレクトリ（書込オープン不可）→false。
- `resolve_generation_suffix`:
  - 全要素が非存在/書込可 → `""`。
  - 1要素を「同名ディレクトリ」にして書込不可にする → `"_1"`。
  - `_1` 版の一部を先に作成 → `"_2"`。
- `generate_srt`/`generate_fcpxml`/`generate_combined_audio` に `"_2"` を渡すと
  `subtitles_2.srt`/`timeline_2.fcpxml`/`full_dialogue_2.wav` が生成される。
- FCPXML 整合: events の音声パスが `voice_0001_2.wav`（suffix 付き）のとき、生成 XML がその名前を `src=` に含む。
- 既存 `generate_*` テスト群は `""` を渡す形に更新し従来どおり通す。

統合（任意・スモーク）: 既定の音声 or 出力を書けない状況で実行すると、`_1` 一式が生成され FCPXML が
`voice_*_1.wav` を参照する。

## 受け入れ条件

- 生成出力のいずれかが使用中のとき、エラー終了せず、個別音声＋SRT＋FCPXML＋統合音声が**共通の `_N`** で保存され、
  FCPXML は同じ `_N` の音声を参照する。WARNING が出る。
- すべて書ける通常時は従来どおり既定名で保存（回帰なし）。
- 既存の出力フォーマット・内容は不変。全テスト通過。`cargo build && cargo test --workspace` 成功。
