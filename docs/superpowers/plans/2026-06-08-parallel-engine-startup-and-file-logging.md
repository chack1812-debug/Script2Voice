# エンジン並行起動 + ログのファイル出力 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 複数音声合成エンジンの自動起動を逐次から並行（fail-fast）に変え、実行ログを `project_dir/run.log` へ追記出力する。

**Architecture:** `EngineManager::activate_required` を `futures::future::try_join_all` で並行化する。ログは `tracing-appender` の非ブロッキングライターと `tracing-subscriber` のレイヤー構成で、コンソールと追記ファイルの 2 系統に出力する。タイムスタンプは `ChronoLocal` でローカル時刻にする。

**Tech Stack:** Rust / tokio / async-trait / futures / tracing / tracing-subscriber / tracing-appender / chrono

設計書: `docs/superpowers/specs/2026-06-08-parallel-engine-startup-and-file-logging-design.md`

---

## Task 1: エンジン並行起動（fail-fast）

`futures` 依存を追加し、`activate_required` を逐次ループから `try_join_all` に置き換える。
並行に走っていることを所要時間で検証するテストを先に書く。

**Files:**
- Modify: `crates/s2v-engines/Cargo.toml`（`[dependencies]` に `futures` 追加）
- Modify: `crates/s2v-engines/src/engine.rs`（`activate_required` の実装、テスト用 `StubEngine` に遅延機能追加、並行性テスト追加）

- [ ] **Step 1: `futures` 依存を追加する**

`crates/s2v-engines/Cargo.toml` の `[dependencies]` セクション（`tracing.workspace = true` の行の下）に追記する。

```toml
futures = "0.3"
```

- [ ] **Step 2: `StubEngine` に遅延機能を追加する（テスト下準備）**

`crates/s2v-engines/src/engine.rs` のテストモジュール内 `StubEngine` を、遅延を設定できるように変更する。

まず構造体定義（現在の `struct StubEngine { ... should_fail: bool, }`）に `delay_ms` フィールドを足す:

```rust
    struct StubEngine {
        activate_count: Arc<AtomicUsize>,
        synthesize_count: Arc<AtomicUsize>,
        terminate_count: Arc<AtomicUsize>,
        should_fail: bool,
        delay_ms: u64,
    }
```

`new()` に `delay_ms: 0` を追加し、遅延付きコンストラクタを足す（`failing()` は `..Self::new()` を使っているため変更不要）:

```rust
    impl StubEngine {
        fn new() -> Self {
            Self {
                activate_count: Arc::new(AtomicUsize::new(0)),
                synthesize_count: Arc::new(AtomicUsize::new(0)),
                terminate_count: Arc::new(AtomicUsize::new(0)),
                should_fail: false,
                delay_ms: 0,
            }
        }

        fn failing() -> Self {
            Self { should_fail: true, ..Self::new() }
        }

        fn with_delay(ms: u64) -> Self {
            Self { delay_ms: ms, ..Self::new() }
        }
    }
```

`activate()` の先頭に遅延を挿入する（`fetch_add` の前後どちらでもよいが、ここでは数え上げの後に sleep）:

```rust
        async fn activate(&self) -> anyhow::Result<()> {
            self.activate_count.fetch_add(1, Ordering::SeqCst);
            if self.delay_ms > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
            }
            if self.should_fail {
                anyhow::bail!("engine activation failed");
            }
            Ok(())
        }
```

- [ ] **Step 3: 並行性を検証する失敗テストを書く**

`crates/s2v-engines/src/engine.rs` のテストモジュール内（`activate_required_propagates_error` の後）に追加する。

```rust
    #[tokio::test]
    async fn activate_required_runs_engines_concurrently() {
        let mut mgr = EngineManager::new();
        mgr.register("voicevox", Arc::new(StubEngine::with_delay(200)));
        mgr.register("aivis", Arc::new(StubEngine::with_delay(200)));

        let types: HashSet<String> = ["voicevox".to_string(), "aivis".to_string()].into();

        let start = std::time::Instant::now();
        mgr.activate_required(&types).await.unwrap();
        let elapsed = start.elapsed();

        // 逐次なら 400ms 以上かかる。並行なら ~200ms。余裕をみて 350ms 未満を要求する。
        assert!(
            elapsed < std::time::Duration::from_millis(350),
            "並行起動なら 350ms 未満で終わるはず。実測: {elapsed:?}"
        );
    }
```

- [ ] **Step 4: テストを実行して失敗を確認する**

Run: `cargo test -p s2v-engines activate_required_runs_engines_concurrently`
Expected: FAIL（現在の逐次実装では elapsed が ~400ms になりアサーション失敗）

- [ ] **Step 5: `activate_required` を並行化する**

`crates/s2v-engines/src/engine.rs` の `activate_required`（現在の `for name in types { ... }` ループ）を置き換える。

```rust
    pub async fn activate_required(&self, types: &HashSet<String>) -> anyhow::Result<()> {
        let engines: Vec<Arc<dyn Engine>> = types
            .iter()
            .filter_map(|name| self.engines.get(name.as_str()).cloned())
            .collect();
        futures::future::try_join_all(engines.iter().map(|e| e.activate())).await?;
        Ok(())
    }
```

- [ ] **Step 6: 並行性テストとエンジン関連テストを実行して通ることを確認する**

Run: `cargo test -p s2v-engines`
Expected: PASS（`activate_required_runs_engines_concurrently` を含む全テストが通る。特に既存の `activate_required_propagates_error`（fail-fast）/ `activate_required_calls_activate_for_matching_engines` / `activate_required_skips_unregistered_engine` が引き続き通る）

- [ ] **Step 7: コミットする**

```bash
git add crates/s2v-engines/Cargo.toml crates/s2v-engines/src/engine.rs
git commit -m "feat(engines): start required engines concurrently with try_join_all"
```

---

## Task 2: ログのファイル追記出力

ルート crate に `tracing-appender` / `chrono` 依存を足し、`tracing-subscriber` に `chrono` feature を足す。
`main` を再構成して `project_dir/run.log` への追記出力を追加する。
純粋関数 `log_file_path` の単体テストを先に書く。

**Files:**
- Modify: `Cargo.toml`（ルート。`tracing-subscriber` の features 変更、`tracing-appender`・`chrono` 追加）
- Modify: `src/main.rs`（`log_file_path` / `init_logging` 追加、`main` 再構成、テスト追加）

- [ ] **Step 1: 依存を追加・変更する**

ルート `Cargo.toml` の `[dependencies]` を編集する。`tracing-subscriber` の行を次のように変更し、2 行を追加する。

```toml
tracing-subscriber = { version = "0.3", features = ["env-filter", "fmt", "chrono"] }
tracing-appender = "0.2"
chrono = "0.4"
```

- [ ] **Step 2: `log_file_path` の失敗テストを書く**

`src/main.rs` の `mod tests`（`fails_without_script_argument` の後）に追加する。

```rust
    #[test]
    fn log_file_path_is_run_log_in_project_dir() {
        let p = log_file_path(std::path::Path::new("/tmp/proj"));
        assert_eq!(p.file_name().unwrap(), "run.log");
        assert_eq!(p.parent().unwrap(), std::path::Path::new("/tmp/proj"));
    }
```

- [ ] **Step 3: テストを実行して失敗を確認する**

Run: `cargo test -p script2voice --bin script2voice log_file_path_is_run_log_in_project_dir`
Expected: FAIL（`log_file_path` が未定義でコンパイルエラー）

- [ ] **Step 4: `log_file_path` と `init_logging` を実装する**

`src/main.rs` の import 群（先頭）に追加する。

```rust
use anyhow::Context;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{fmt, EnvFilter};
```

（注：既存の `use anyhow::Context;` と `use tracing_subscriber::EnvFilter;` がある場合は重複させない。
`EnvFilter` は `tracing_subscriber::{fmt, EnvFilter}` の一括 import にまとめ、旧 `use tracing_subscriber::EnvFilter;` 行は削除する。）

`main` 関数の手前に 2 つの関数を追加する。

```rust
/// 実行ログを追記するファイルのパス（project_dir/run.log）を返す。
fn log_file_path(project_dir: &std::path::Path) -> PathBuf {
    project_dir.join("run.log")
}

/// コンソール（stdout）と project_dir/run.log の両方へログを出力する subscriber を初期化する。
/// 返した WorkerGuard は main 終了まで保持すること（drop でバッファを flush する）。
fn init_logging(project_dir: &std::path::Path) -> anyhow::Result<WorkerGuard> {
    let path = log_file_path(project_dir);
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("ログファイルを開けません: {}", path.display()))?;
    let (file_writer, guard) = tracing_appender::non_blocking(file);

    let time_format = "%Y-%m-%d %H:%M:%S%.3f".to_string();

    let console_layer = fmt::layer()
        .with_timer(ChronoLocal::new(time_format.clone()))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    let file_layer = fmt::layer()
        .with_ansi(false)
        .with_writer(file_writer)
        .with_timer(ChronoLocal::new(time_format))
        .with_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")));

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .init();

    Ok(guard)
}
```

- [ ] **Step 5: テストを実行して通ることを確認する**

Run: `cargo test -p script2voice --bin script2voice log_file_path_is_run_log_in_project_dir`
Expected: PASS

- [ ] **Step 6: `main` を再構成してログ初期化を `project_dir` 確定後に移す**

`src/main.rs` の `main` 関数を編集する。先頭の `tracing_subscriber::fmt()....init();` ブロックを削除し、`create_dir_all` の直後に `init_logging` 呼び出しを置く。変更後の `main` 前半は次のようになる。

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    let script_path = std::fs::canonicalize(&cli.script)
        .with_context(|| format!("台本ファイルが見つかりません: {}", cli.script.display()))?;

    let project_name = script_path
        .file_stem()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow::anyhow!("台本ファイル名が不正です: {}", script_path.display()))?
        .to_string();
    let project_dir = script_path
        .parent()
        .unwrap_or_else(|| std::path::Path::new("."))
        .join(&project_name);
    std::fs::create_dir_all(&project_dir)?;

    let _guard = init_logging(&project_dir)?;

    tracing::info!("--- Project: {project_name} ---");
    tracing::info!("Output Directory: {}", project_dir.display());

    // 以降（exe_path / config 読み込み以降）は既存のまま変更しない。
```

`main` の残り（`let exe_path = ...` 以降）は変更しない。`_guard` は `main` のスコープ末尾まで生存し、ログのバッファを最後に flush する。

- [ ] **Step 7: ビルドと全テストを実行して通ることを確認する**

Run: `cargo build && cargo test`
Expected: PASS（ワークスペース全体がビルドでき、全テストが通る）

- [ ] **Step 8: 実機でログファイル出力を手動確認する**

Run: `cargo run -- <任意の台本.txt>`（エンジン未設定でも、`run.log` の生成と起動直後のログ出力までは確認できる）
Expected: 台本と同名の出力フォルダ内に `run.log` が作成され、`--- Project: ... ---` などのログがローカル時刻付きで記録されている。続けてもう一度実行すると同じ `run.log` に追記される。

- [ ] **Step 9: コミットする**

```bash
git add Cargo.toml src/main.rs
git commit -m "feat(logging): append run logs to project_dir/run.log alongside console"
```

---

## Self-Review メモ

- **Spec coverage**: Part A（並行起動 fail-fast）= Task 1。Part B（run.log 追記・ChronoLocal・依存追加・main 再構成・log_file_path テスト）= Task 2。設計書の受け入れ条件（並行性／fail-fast 維持／run.log 追記／既存テスト通過）を各タスクのステップで網羅。
- **Placeholder scan**: プレースホルダなし。各コードステップに実コードを記載。
- **Type consistency**: `log_file_path(&Path) -> PathBuf` / `init_logging(&Path) -> anyhow::Result<WorkerGuard>` / `StubEngine::with_delay(u64)` は定義箇所と使用箇所で一致。
