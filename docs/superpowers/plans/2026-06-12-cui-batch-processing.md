# CUI 複数台本バッチ処理 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** CUI（`script2voice`）が複数台本（ファイル/フォルダ混在）を受け取り、エンジンを1回だけ起動して全台本を暖まったまま処理し、エラーがあっても継続して、最後にエンジンを停止する。

**Architecture:** `src/main.rs` のバイナリに、引数展開・事前パース・必要エンジン union 算出・継続ループ・差し替え可能ログ出力の小さな自由関数群を追加し、`main` がそれらを配線する。台本ごとの処理は既存 `Producer::produce`（`src/lib.rs`）を無改修で再利用する。並行処理ロジックや `s2v-engines` には触れない。

**Tech Stack:** Rust 2021 / tokio / clap v4 (derive) / tracing-subscriber 0.3 / anyhow / tempfile（dev）。

設計書: `docs/superpowers/specs/2026-06-12-cui-batch-processing-design.md`

---

## File Structure

- **Modify** `src/main.rs` — 唯一の変更ファイル。以下を追加/変更する:
  - `Cli.script: PathBuf` → `Cli.scripts: Vec<PathBuf>`
  - `expand_script_args`（新規・引数展開）
  - `parse_all`（新規・継続パース）
  - `required_engines`（新規・必要エンジン union）
  - `SharedLogFile` / `SharedLogWriter` + `init_logging` 改修 + `open_run_log`（新規・差し替えログ）
  - `BatchSummary` + `run_each`（新規・継続ループ、クロージャ注入でテスト可能）
  - `activate_each`（新規・個別エンジン起動・継続）
  - `process_one`（新規・1台本処理＝project_dir決定＋ログ切替＋`Producer.produce`）
  - `main` 書き換え（配線・サマリ・終了コード）
  - 既存 `log_file_path` / `init_logging(project_dir)` / `run_pipeline` を置換
- `src/lib.rs`（`Producer`）: **変更なし**
- `crates/s2v-engines/*`: **変更なし**
- `Cargo.toml`: **変更なし**（`tracing-appender` は未使用化するが依存は残す）

各タスクは `cargo test -p script2voice <名前>` で個別に検証する。

---

## Task 1: 引数展開 `expand_script_args`

**Files:**
- Modify: `src/main.rs`（自由関数追加 + `#[cfg(test)]` にテスト追加）

- [ ] **Step 1: 失敗するテストを書く**

`src/main.rs` の `mod tests` 内に追加:

```rust
    #[test]
    fn expand_collects_txt_in_directory_sorted() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("b.txt"), "x").unwrap();
        std::fs::write(dir.path().join("a.txt"), "x").unwrap();
        std::fs::write(dir.path().join("note.md"), "x").unwrap();
        let out = expand_script_args(&[dir.path().to_path_buf()]).unwrap();
        let names: Vec<String> = out
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, vec!["a.txt", "b.txt"]);
    }

    #[test]
    fn expand_accepts_files_and_dedups() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("s.txt");
        std::fs::write(&f, "x").unwrap();
        let out = expand_script_args(&[f.clone(), f.clone()]).unwrap();
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn expand_errors_on_missing_path() {
        let res = expand_script_args(&[PathBuf::from("definitely_not_here_12345.txt")]);
        assert!(res.is_err());
    }

    #[test]
    fn expand_errors_when_directory_has_no_txt() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.md"), "x").unwrap();
        let res = expand_script_args(&[dir.path().to_path_buf()]);
        assert!(res.is_err());
    }
```

- [ ] **Step 2: テストが失敗（コンパイルエラー）することを確認**

Run: `cargo test -p script2voice expand_ 2>&1 | tail -20`
Expected: FAIL（`cannot find function expand_script_args`）

- [ ] **Step 3: 最小実装を書く**

`src/main.rs` のファイル冒頭の `use` を更新（不足ぶんを追加）:

```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};
```

`run_pipeline` の前あたりに自由関数を追加:

```rust
/// 台本引数を展開する。
/// - ファイル: そのまま採用。
/// - ディレクトリ: 直下の拡張子 `.txt`（大文字小文字無視）を名前順に採用（再帰しない）。
/// - 存在しないパス: エラー。
/// 採用後に canonicalize した実体パスで重複を除去（出現順は維持）。0 件ならエラー。
fn expand_script_args(args: &[PathBuf]) -> anyhow::Result<Vec<PathBuf>> {
    fn push_unique(p: &Path, out: &mut Vec<PathBuf>, seen: &mut HashSet<PathBuf>) -> anyhow::Result<()> {
        let canon = std::fs::canonicalize(p)
            .with_context(|| format!("パスを解決できません: {}", p.display()))?;
        if seen.insert(canon.clone()) {
            out.push(canon);
        }
        Ok(())
    }

    let mut out: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();

    for arg in args {
        if arg.is_dir() {
            let mut entries: Vec<PathBuf> = std::fs::read_dir(arg)
                .with_context(|| format!("フォルダを読めません: {}", arg.display()))?
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| {
                    p.is_file()
                        && p.extension()
                            .and_then(|e| e.to_str())
                            .map(|e| e.eq_ignore_ascii_case("txt"))
                            .unwrap_or(false)
                })
                .collect();
            entries.sort();
            for p in entries {
                push_unique(&p, &mut out, &mut seen)?;
            }
        } else if arg.is_file() {
            push_unique(arg, &mut out, &mut seen)?;
        } else {
            anyhow::bail!("台本パスが見つかりません: {}", arg.display());
        }
    }

    if out.is_empty() {
        anyhow::bail!("処理対象の台本が見つかりません（.txt が 0 件）");
    }
    Ok(out)
}
```

> 注: `use anyhow::Context;` は既存の `use` に含まれている（`with_context` 用）。`HashSet` は既存で import 済みだが、`Path` の追加が必要。

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p script2voice expand_ 2>&1 | tail -20`
Expected: PASS（4 件）

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): expand script args (files/dirs) for batch processing"
```

---

## Task 2: 必要エンジン union `required_engines`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 失敗するテストを書く**

`mod tests` に追加:

```rust
    #[test]
    fn required_engines_unions_across_scripts() {
        let mut p1 = s2v_core::ScriptParser::new();
        let s1 = p1
            .parse_str("@scene S\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:あ\n")
            .unwrap();
        let mut p2 = s2v_core::ScriptParser::new();
        let s2 = p2
            .parse_str("@scene S\n@cast\nB:話者:ノーマル,aivis,pan=0\n@script\nB:い\n")
            .unwrap();
        let parsed = vec![(PathBuf::from("1.txt"), s1), (PathBuf::from("2.txt"), s2)];
        let req = required_engines(&parsed);
        assert!(req.contains("voicevox"));
        assert!(req.contains("aivis"));
        assert_eq!(req.len(), 2);
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p script2voice required_engines 2>&1 | tail -20`
Expected: FAIL（`cannot find function required_engines` / `Scene` 未解決）

- [ ] **Step 3: 最小実装を書く**

ファイル冒頭の `use s2v_core::{Config, ScriptParser};` を以下に変更:

```rust
use s2v_core::{Config, Scene, ScriptParser};
```

自由関数を追加:

```rust
/// パース済み全台本から、使用される engine_type の和集合を作る。
fn required_engines(parsed: &[(PathBuf, Vec<Scene>)]) -> HashSet<String> {
    let mut set = HashSet::new();
    for (_, scenes) in parsed {
        for scene in scenes {
            for cast in scene.casts.values() {
                set.insert(cast.engine_type.clone());
            }
        }
    }
    set
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p script2voice required_engines 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): compute required-engine union across scripts"
```

---

## Task 3: 継続パース `parse_all`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 失敗するテストを書く**

`mod tests` に追加:

```rust
    #[test]
    fn parse_all_continues_past_unreadable_file() {
        let dir = tempfile::tempdir().unwrap();
        let good = dir.path().join("good.txt");
        std::fs::write(
            &good,
            "@scene S\n@cast\nA:話者:ノーマル,voicevox,pan=0\n@script\nA:こんにちは\n",
        )
        .unwrap();
        let bad = dir.path().join("bad.txt");
        std::fs::write(&bad, [0xff, 0xfe, 0x00, 0x01]).unwrap(); // 不正UTF-8 → read_to_string失敗

        let (parsed, failures) = parse_all(&[good.clone(), bad.clone()]);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].0, good);
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, bad);
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p script2voice parse_all 2>&1 | tail -20`
Expected: FAIL（`cannot find function parse_all`）

- [ ] **Step 3: 最小実装を書く**

自由関数を追加:

```rust
/// 各台本をパースする。失敗しても止めず、成功分と失敗分(パス, 理由)を分けて返す。
/// （`parse_file` はファイル読み込み/UTF-8 エラー時のみ Err。書式の崩れは警告扱いで Ok。）
fn parse_all(scripts: &[PathBuf]) -> (Vec<(PathBuf, Vec<Scene>)>, Vec<(PathBuf, String)>) {
    let mut parsed = Vec::new();
    let mut failures = Vec::new();
    for path in scripts {
        let mut parser = ScriptParser::new();
        match parser.parse_file(path) {
            Ok(scenes) => parsed.push((path.clone(), scenes)),
            Err(e) => {
                tracing::error!("パース失敗 {}: {e:#}", path.display());
                failures.push((path.clone(), format!("パース失敗: {e:#}")));
            }
        }
    }
    (parsed, failures)
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p script2voice parse_all 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): parse all scripts up front, continue on parse error"
```

---

## Task 4: 差し替え可能ログ出力（台本ごと run.log）

**Files:**
- Modify: `src/main.rs`（`SharedLogFile` 追加、`init_logging` 改修、`log_file_path` 削除、`open_run_log` 追加）

- [ ] **Step 1: 失敗するテストを書く**

`mod tests` に追加:

```rust
    #[test]
    fn shared_log_writer_discards_when_unset_and_writes_when_set() {
        use std::io::Write;
        use tracing_subscriber::fmt::MakeWriter;

        let shared = SharedLogFile::default();

        // 未設定: 書いても捨てられ、エラーにならない
        {
            let mut w = shared.make_writer();
            assert!(w.write(b"discarded\n").is_ok());
        }

        // 設定: ファイルに書かれる
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("run.log");
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .unwrap();
        shared.set(Some(file));
        {
            let mut w = shared.make_writer();
            w.write_all(b"hello-log\n").unwrap();
            w.flush().unwrap();
        }
        shared.set(None);

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("hello-log"));
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p script2voice shared_log_writer 2>&1 | tail -20`
Expected: FAIL（`cannot find type SharedLogFile`）

- [ ] **Step 3: 最小実装を書く**

ファイル冒頭の `use std::sync::Arc;` を以下に変更:

```rust
use std::sync::{Arc, Mutex};
```

`tracing_appender` 関連の import を削除する（未使用化するため）。具体的には次の行を削除:

```rust
use tracing_appender::non_blocking::WorkerGuard;
```

既存の `log_file_path` 関数と、既存の `init_logging(project_dir) -> WorkerGuard` 関数を**丸ごと削除**し、以下に置き換える:

```rust
/// 現在処理中の台本の run.log を指す差し替え可能なファイルハンドル。
/// バッチでは subscriber を1つだけ使い回し、台本の境界でこのハンドルを差し替える。
#[derive(Clone, Default)]
struct SharedLogFile(Arc<Mutex<Option<std::fs::File>>>);

impl SharedLogFile {
    /// ファイル出力先を設定する（None でファイル出力を止める）。
    fn set(&self, file: Option<std::fs::File>) {
        *self.0.lock().unwrap() = file;
    }
}

/// `SharedLogFile` が現在指すファイルへ書き込む Writer。未設定時は破棄する。
struct SharedLogWriter(Arc<Mutex<Option<std::fs::File>>>);

impl std::io::Write for SharedLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut guard = self.0.lock().unwrap();
        match guard.as_mut() {
            Some(f) => f.write(buf),
            None => Ok(buf.len()), // 出力先未設定時は破棄
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        let mut guard = self.0.lock().unwrap();
        match guard.as_mut() {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for SharedLogFile {
    type Writer = SharedLogWriter;
    fn make_writer(&'a self) -> Self::Writer {
        SharedLogWriter(self.0.clone())
    }
}

/// コンソール + 差し替え可能ファイルの subscriber をプロセスで1回だけ初期化する。
/// 返した `SharedLogFile` を台本の境界で `set` してファイル出力先を切り替える。
fn init_logging() -> SharedLogFile {
    let shared = SharedLogFile::default();
    let time_format = "%Y-%m-%d %H:%M:%S%.3f".to_string();

    let console_layer = fmt::layer()
        .with_timer(ChronoLocal::new(time_format.clone()))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(shared.clone())
        .with_timer(ChronoLocal::new(time_format))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    shared
}

/// 台本の出力フォルダに run.log を追記オープンする。
fn open_run_log(project_dir: &Path) -> anyhow::Result<std::fs::File> {
    let path = project_dir.join("run.log");
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))
}
```

既存の `mod tests` にある `log_file_path_is_run_log_in_project_dir` テストを**削除**する（`log_file_path` 関数を消したため）。

> このタスクは `main` の呼び出し側をまだ直していないためコンパイルが通らない場合がある。次の Step でテスト対象だけを確認し、`main` の結線は Task 7 で行う。

- [ ] **Step 4: テストが通ることを確認**

`main` 内の旧 `init_logging(&project_dir)?` 呼び出しがまだ残っていてコンパイルが通らない場合は、その行を一時的に `let _log_file = init_logging();` に置き換えてビルドを通す（正式な配線は Task 7）。その上で:

Run: `cargo test -p script2voice shared_log_writer 2>&1 | tail -20`
Expected: PASS

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(log): swappable per-script run.log writer (one subscriber)"
```

---

## Task 5: 継続ループ `run_each` と `BatchSummary`

**Files:**
- Modify: `src/main.rs`

- [ ] **Step 1: 失敗するテストを書く**

`mod tests` に追加:

```rust
    #[tokio::test]
    async fn run_each_continues_after_failure_and_counts() {
        let mut parser = s2v_core::ScriptParser::new();
        let scenes = parser.parse_str("@scene S\n@script\n").unwrap();
        let parsed = vec![
            (PathBuf::from("a.txt"), scenes.clone()),
            (PathBuf::from("b.txt"), scenes.clone()),
            (PathBuf::from("c.txt"), scenes.clone()),
        ];

        let summary = run_each(parsed, Vec::new(), |path, _scenes| async move {
            if path == PathBuf::from("b.txt") {
                anyhow::bail!("わざと失敗")
            } else {
                Ok(())
            }
        })
        .await;

        assert_eq!(summary.succeeded, 2);
        assert_eq!(summary.failures.len(), 1);
        assert_eq!(summary.failures[0].0, PathBuf::from("b.txt"));
        assert!(summary.has_failure());
        assert_eq!(summary.total(), 3);
    }

    #[tokio::test]
    async fn run_each_keeps_prior_failures() {
        let summary = run_each(
            Vec::new(),
            vec![(PathBuf::from("parsefail.txt"), "パース失敗".to_string())],
            |_p, _s| async { Ok(()) },
        )
        .await;
        assert_eq!(summary.succeeded, 0);
        assert_eq!(summary.failures.len(), 1);
        assert!(summary.has_failure());
    }
```

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p script2voice run_each 2>&1 | tail -20`
Expected: FAIL（`cannot find function run_each` / `BatchSummary`）

- [ ] **Step 3: 最小実装を書く**

自由関数と型を追加:

```rust
/// バッチ処理の結果サマリ。
struct BatchSummary {
    succeeded: usize,
    failures: Vec<(PathBuf, String)>,
}

impl BatchSummary {
    fn total(&self) -> usize {
        self.succeeded + self.failures.len()
    }
    fn has_failure(&self) -> bool {
        !self.failures.is_empty()
    }
}

/// パース済み台本を順に処理する。各台本の失敗で止めず、`prior_failures`（パース失敗等）に
/// 積み増してサマリを返す。`process` は1台本ぶんの処理（成功で Ok、失敗で Err）。
/// テスト容易化のためクロージャ注入にしている。
async fn run_each<F, Fut>(
    parsed: Vec<(PathBuf, Vec<Scene>)>,
    mut prior_failures: Vec<(PathBuf, String)>,
    process: F,
) -> BatchSummary
where
    F: Fn(PathBuf, Vec<Scene>) -> Fut,
    Fut: std::future::Future<Output = anyhow::Result<()>>,
{
    let total = parsed.len();
    let mut succeeded = 0usize;
    for (i, (path, scenes)) in parsed.into_iter().enumerate() {
        tracing::info!("[{}/{}] 処理開始: {}", i + 1, total, path.display());
        match process(path.clone(), scenes).await {
            Ok(()) => {
                succeeded += 1;
                tracing::info!("[{}/{}] 完了: {}", i + 1, total, path.display());
            }
            Err(e) => {
                tracing::error!("処理失敗 {}: {e:#}", path.display());
                prior_failures.push((path, format!("{e:#}")));
            }
        }
    }
    BatchSummary { succeeded, failures: prior_failures }
}
```

- [ ] **Step 4: テストが通ることを確認**

Run: `cargo test -p script2voice run_each 2>&1 | tail -20`
Expected: PASS（2 件）

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): continue-on-error batch loop with summary"
```

---

## Task 6: エンジン個別起動 `activate_each` と1台本処理 `process_one`

**Files:**
- Modify: `src/main.rs`

> この2関数は実エンジン/実合成に依存するため、専用の自動テストは置かない（`run_each` の
> 継続テストと Task 8 の手動確認でカバー）。コンパイルが通り、`main` から呼べることを確認する。

- [ ] **Step 1: 実装を書く**

ファイル冒頭の `use s2v_engines::EngineManager;` を以下に変更（`Engine` トレイトを `.activate()` のため取り込む）:

```rust
use s2v_engines::{Engine, EngineManager};
```

自由関数を追加:

```rust
/// 必要エンジンを個別に起動する。1つの失敗で全体を止めず、警告して継続する
/// （起動に失敗したエンジンを使う台本は後段の合成で失敗扱いになる）。
async fn activate_each(engine_manager: &Arc<EngineManager>, required: &HashSet<String>) {
    for name in required {
        let Some(engine) = engine_manager.get(name) else {
            tracing::warn!("[{name}] 未登録のエンジンが要求されました。スキップします。");
            continue;
        };
        match engine.activate().await {
            Ok(()) => tracing::info!("[{name}] エンジン起動完了。"),
            Err(e) => tracing::warn!(
                "[{name}] エンジン起動に失敗しました（このエンジンを使う台本は失敗します）: {e:#}"
            ),
        }
    }
}

/// 1台本を処理する。出力フォルダ（台本名）を決め、その run.log にログを向けてから
/// 既存 `Producer` を実行する。ログ出力先は処理後に必ず外す。
async fn process_one(
    script_path: &Path,
    scenes: &[Scene],
    config: &Config,
    engine_manager: &Arc<EngineManager>,
    log_file: &SharedLogFile,
) -> anyhow::Result<()> {
    let project_name = script_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("台本ファイル名が不正です: {}", script_path.display()))?
        .to_string();
    let project_dir = script_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&project_name);
    std::fs::create_dir_all(&project_dir)?;

    log_file.set(Some(open_run_log(&project_dir)?));
    let result = async {
        tracing::info!("--- Project: {project_name} ---");
        tracing::info!("Output Directory: {}", project_dir.display());
        let producer = Producer::new(Arc::clone(engine_manager), config, &project_dir)?;
        producer.produce(scenes).await?;
        tracing::info!("--- 完了: {project_name} ---");
        anyhow::Ok(())
    }
    .await;
    log_file.set(None);
    result
}
```

- [ ] **Step 2: コンパイルを確認**

> `main` がまだ旧構造のため、この時点では未使用警告やコンパイルエラーが出てよい。型チェックだけ先に通すため、`main` 結線は Task 7。ここでは `cargo check` で新関数自体に型エラーがないことを見る。

Run: `cargo check -p script2voice 2>&1 | tail -30`
Expected: `process_one` / `activate_each` 自体に型エラーがないこと（`main` の旧コード由来のエラー・未使用警告は許容、Task 7 で解消）

- [ ] **Step 3: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): per-script processing and tolerant engine activation"
```

---

## Task 7: `main` 書き換え（配線・終了コード）と既存 CLI テスト更新

**Files:**
- Modify: `src/main.rs`（`Cli`、`main`、`run_pipeline` 削除、既存テスト更新）

- [ ] **Step 1: 失敗するテストを書く（CLI を複数台本前提に更新）**

`mod tests` 内の既存テストを次のように差し替える:

- `parses_script_path_with_no_explicit_config` を削除し、以下を追加:

```rust
    #[test]
    fn parses_multiple_script_paths() {
        let cli = Cli::try_parse_from(["script2voice", "a.txt", "b.txt"]).unwrap();
        assert_eq!(cli.scripts, vec![PathBuf::from("a.txt"), PathBuf::from("b.txt")]);
        assert_eq!(cli.config, None);
    }
```

- `parses_custom_config_path` は内容そのままで可（`scripts` に1件入り `config` が取れることを確認）。ただしアサーションを `cli.config` のみ参照する形に保つ（既にそうなっている）。

- `fails_without_script_argument` はそのまま（引数0でエラー）。

- [ ] **Step 2: テストが失敗することを確認**

Run: `cargo test -p script2voice parses_multiple_script_paths 2>&1 | tail -20`
Expected: FAIL（`Cli` に `scripts` が無い / `script` 参照のコンパイルエラー）

- [ ] **Step 3: `Cli`・`main` を実装し、`run_pipeline` を削除**

`Cli` を変更:

```rust
#[derive(Parser)]
#[command(name = "script2voice", version, about = "台本から音声・字幕・タイムラインを生成する")]
struct Cli {
    /// 台本ファイルまたはフォルダ（複数指定可。フォルダは直下の .txt を名前順に処理）
    #[arg(required = true, num_args = 1..)]
    scripts: Vec<PathBuf>,

    /// 設定ファイル (config.toml) のパス。省略時は実行ファイルと同じディレクトリの config.toml を使用する
    #[arg(short, long)]
    config: Option<PathBuf>,
}
```

`main` を全面的に書き換える:

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let log_file = init_logging();

    let scripts = expand_script_args(&cli.scripts)?;
    tracing::info!("処理対象: {} 台本", scripts.len());

    let exe_path = std::env::current_exe().ok();
    let config_path = resolve_config_path(cli.config.clone(), exe_path.as_deref());
    tracing::info!("設定ファイル: {}", config_path.display());
    let config = Config::from_file(&config_path)
        .with_context(|| format!("設定ファイルの読み込みに失敗しました: {}", config_path.display()))?;

    // 事前パース（失敗は継続）
    let (parsed, parse_failures) = parse_all(&scripts);

    // 必要エンジンを1回だけ起動（失敗は継続）
    let required = required_engines(&parsed);
    tracing::info!(
        "使用予定のエンジン: {}",
        required.iter().cloned().collect::<Vec<_>>().join(", ")
    );
    let engine_manager = Arc::new(build_engine_manager(&config));
    activate_each(&engine_manager, &required).await;

    // 台本ごとに処理（暖まったエンジンを使い回し、失敗は継続）
    let config_ref = &config;
    let em_ref = &engine_manager;
    let log_ref = &log_file;
    let summary = run_each(parsed, parse_failures, |path, scenes| async move {
        process_one(&path, &scenes, config_ref, em_ref, log_ref).await
    })
    .await;

    // 全台本終了後にエンジンを停止
    engine_manager.shutdown_all();

    // サマリ
    tracing::info!(
        "=== バッチ完了: 成功 {} / 失敗 {} (合計 {}) ===",
        summary.succeeded,
        summary.failures.len(),
        summary.total()
    );
    for (path, reason) in &summary.failures {
        tracing::warn!("失敗: {} — {}", path.display(), reason);
    }
    if summary.has_failure() {
        anyhow::bail!("{} 台本が失敗しました", summary.failures.len());
    }
    Ok(())
}
```

`run_pipeline` 関数を**削除**する（`main` に吸収済み）。

ファイル冒頭の `use` を最終形に整える（不足や未使用を解消）:

```rust
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use clap::Parser;
use s2v_core::{Config, Scene, ScriptParser};
use s2v_engines::{Engine, EngineManager};
use script2voice::{build_engine_manager, resolve_config_path, Producer};
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};
```

> `tracing_appender` と `WorkerGuard` の import は削除済みであること。`run_pipeline` で使っていた
> 旧 import が残っていれば消す。

- [ ] **Step 4: テスト・ビルドが通ることを確認**

Run: `cargo test -p script2voice 2>&1 | tail -30`
Expected: PASS（全テスト）。警告0が望ましいが、最低限エラーなし。

- [ ] **Step 5: コミット**

```bash
git add src/main.rs
git commit -m "feat(cli): wire batch pipeline in main, stop engines after all scripts"
```

---

## Task 8: 全体検証・手動確認・教訓記録

**Files:**
- 変更なし（検証のみ）。記録は Obsidian。

- [ ] **Step 1: ワークスペース全体のテスト**

Run: `cargo test 2>&1 | tail -30`
Expected: 全クレートのテストが PASS。

- [ ] **Step 2: clippy（任意だが推奨）**

Run: `cargo clippy -p script2voice 2>&1 | tail -30`
Expected: 重大な警告なし（既存水準を維持）。

- [ ] **Step 3: 手動確認（複数台本バッチ）**

リポジトリ同梱のサンプル台本フォルダを使う。実エンジン（VOICEVOX/AivisSpeech）が必要。

Run: `cargo run -p script2voice -- "scripts/音響テスト" 2>&1 | tail -40`
Expected:
- 起動時に「処理対象: N 台本」「使用予定のエンジン: ...」が出る。
- エンジン起動ログが**1回だけ**（台本ごとに繰り返さない）。
- 各台本フォルダ（`scripts/音響テスト/<台本名>/`）に `audio/`・`timeline/`・`full_dialogue.wav`・`run.log` が生成される。
- `run.log` が**台本ごとに分かれて**いる。
- 末尾に「=== バッチ完了: 成功 X / 失敗 Y ===」が出る。

複数パス指定・混在も確認:

Run: `cargo run -p script2voice -- "scripts/音響テスト.txt" "scripts/Script2Voice紹介イベント.txt" 2>&1 | tail -40`
Expected: 両台本が1プロセスで処理され、エンジン起動は1回。

- [ ] **Step 4: 失敗・教訓があれば記録**

実装中に遭遇した失敗・ハマりどころがあれば、CLAUDE.md の規約に従い Obsidian に記録する:
- 失敗・教訓: `D:\Obsidianvault\ClaudeMemory\Projects\Script2Voice-Rust版\失敗・教訓ログ.md`（先頭付近に追記）
- 進捗: `D:\Obsidianvault\ClaudeMemory\Projects\Script2Voice-Rust版\進捗.md`

- [ ] **Step 5: 最終コミット（必要なら）**

検証で追加の修正があればコミットする。なければスキップ。

```bash
git status
```

---

## Self-Review（記入済み）

**1. Spec coverage（設計書との対応）**
- Part A（CLI 引数・展開）→ Task 1・Task 7。
- Part B（事前パース・union・1回起動・台本ごと処理・終了時停止・継続・終了コード）→ Task 2/3/5/6/7。
- Part C（台本ごと run.log・差し替え MakeWriter）→ Task 4・Task 6（`process_one` で `set`）。
- テスト（引数展開／union／継続／ログ）→ Task 1/2/3/4/5。設計書の「結合テスト（エンジンはスタブ）」は、
  実合成に依存しない `run_each` のクロージャ注入テスト（Task 5）で「継続＋件数＋失敗判定」を担保する方針に置換（スタブ合成の WAV 生成依存を避けるため）。エンジン込みの end-to-end は Task 8 の手動確認でカバー。
- 「ディレクトリは直下のみ・再帰しない」「エンジン起動は逐次」→ 実装に反映（設計書の未決事項どおり）。

**2. Placeholder scan:** TBD/TODO/「適切に処理」等のプレースホルダなし。各コードステップは完全なコードを記載。

**3. Type consistency:**
- `SharedLogFile` / `SharedLogWriter` / `set` / `make_writer` は Task 4 と Task 6・7 で一致。
- `BatchSummary { succeeded, failures }` / `total()` / `has_failure()` は Task 5・7 で一致。
- `run_each(parsed, prior_failures, process)` の引数・戻り値は Task 5・7 で一致。
- `process_one(script_path, scenes, config, engine_manager, log_file)` の並びは Task 6・7 で一致。
- `required_engines(&[(PathBuf, Vec<Scene>)])` は Task 2・7 で一致。
- import（`Path`/`Mutex`/`Scene`/`Engine`）は各タスクで追加し、Task 7 で最終形に集約。
