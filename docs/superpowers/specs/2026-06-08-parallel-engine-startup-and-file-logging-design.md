# エンジン並行起動 + ログのファイル出力 設計書

作成日: 2026-06-08

## 背景・目的

音声合成エンジン（VOICEVOX / AivisSpeech / XTTS）の自動起動が **逐次** に行われている。
台本で複数エンジンを使い、いずれも未起動の場合、`activate_required` が 1 つずつ
`activate().await` するため、各エンジンの起動待機（最大 30 秒ポーリング）が直列に積み上がる。
2 エンジンなら最悪 60 秒かかる。これを **並行起動** にして最悪でも約 30 秒に短縮する。

あわせて、現状コンソール（stderr）にしか出ていない実行ログを **ファイルにも残す**。
トラブル発生時に後から実行内容を追跡できるようにする。

## スコープ

- **やること**
  - `EngineManager::activate_required` を並行起動（fail-fast）に変更する。
  - 実行ログを `project_dir` 配下のタイムスタンプ付きファイルにも出力する。
- **やらないこと**
  - `ensure_running` 内部（個々のエンジンの起動・ポーリング処理）の変更。
  - ログのローテーション／世代削除／サイズ上限（単一ファイルに追記し続けるだけ）。
  - Python 版への同等変更。

## Part A — エンジン並行起動（fail-fast）

### 対象
`crates/s2v-engines/src/engine.rs` の `activate_required`。

### 変更内容
逐次の `for` ループを `futures::future::try_join_all` による並行実行に置き換える。

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

### 設計判断
- **fail-fast セマンティクス**：`try_join_all` は最初の失敗で即 `Err` を返す。残りの起動中
  プロセスや、すでに起動に成功した他エンジンのプロセスは、`main` の `shutdown_all()`
  （`try/finally` 相当）が後始末するため、リークしない。現状の逐次版と同じく fail-fast。
- **借用の安全性**：`Arc<dyn Engine>` を一旦 `Vec` に集めてから `e.activate()` の future を
  生成する。future は `engines` 内の `Arc` を借用するが、`try_join_all` の完了まで `engines`
  は生存しているため安全。`tokio::spawn`（`'static` 制約）は不要。
- **依存追加**：`s2v-engines` の `[dependencies]` に `futures = "0.3"` を追加。
  既に `Cargo.lock` に解決済み（reqwest/tokio 経由）のため、新規クレート取得は発生しない。

### テスト
- 新規：`StubEngine` に設定可能な遅延（`activate()` 内で一定時間 `sleep`）を持たせ、
  2 エンジンを登録して `activate_required` を呼び、**合計経過時間が逐次実行（遅延×2）より
  十分短い（≈ 遅延×1）** ことを検証する。並行に走っている証拠とする。
- 既存維持：`activate_required_propagates_error`（1 つが失敗したらエラーを伝播する=fail-fast）、
  `activate_required_calls_activate_for_matching_engines`、`activate_required_skips_unregistered_engine`。

## Part B — ログのファイル出力

### 対象
`src/main.rs`、ルート `Cargo.toml`。

### 出力仕様
- 出力先：`project_dir/run.log`（台本ごとの出力フォルダ内の **固定名・単一ファイル**）。
- 世代管理：**追記方式**。実行のたびにこのファイルへ追記し続ける（上書き・ローテーションなし）。
  各ログ行に時刻が入るため、過去の実行も時刻で区別できる。さらに既存の `--- Project: X ---`
  ログ行が各実行の開始マーカーとして機能する。
- タイムスタンプ：各層の timer を `ChronoLocal`（ローカル時刻）にし、`YYYY-MM-DD HH:MM:SS.mmm`
  形式で出力する。コンソール・ファイルとも同一フォーマットで一貫させる。
- ログレベル：コンソールと同じ `EnvFilter`（既定 `info`、環境変数 `RUST_LOG` で上書き可）。
- ファイル層は ANSI カラーコードを出力しない（`.with_ansi(false)`）。

### 依存追加（ルート `Cargo.toml`）
- `tracing-appender = "0.2"`：非ブロッキングなファイルライター（`WorkerGuard` でバッファ flush）。
- `chrono = "0.4"`：ログ行のローカル時刻タイムスタンプ（`ChronoLocal` timer）用。
- `tracing-subscriber` の features に `chrono` を追加（`ChronoLocal` timer を有効化）。
  → `features = ["env-filter", "fmt", "chrono"]`。

### `main` の再構成
ログ初期化を `project_dir` 確定後に移す。順序は次のとおり。

1. CLI 解析 → 台本 `canonicalize` → `project_name` / `project_dir` 算出 → `create_dir_all`。
   （ここまではログ初期化前。この区間で発生したエラーは `anyhow` の `main` 戻り値経由で
   stderr に表示されるため、ログファイルが無くても可視。）
2. ログファイルパスを `log_file_path(project_dir) -> PathBuf`（= `project_dir/run.log`）で構築する。
3. ファイルを **追記モード** で開く（`OpenOptions::new().create(true).append(true).open(path)`）。
   それを `tracing_appender::non_blocking(file)` に渡して `(writer, guard)` を得る。
   `guard` は `main` のスコープ末尾まで束縛し続ける（drop 時にバッファを flush）。
4. `tracing_subscriber::registry()` に **stderr 層** と **ファイル層** の 2 つの `fmt` レイヤーを
   重ねて `.init()` する。各層に個別の `EnvFilter`（既定 `info`／`RUST_LOG` 反映）と
   `ChronoLocal` timer を付ける。ファイル層は `.with_ansi(false)`。
5. 既存の `--- Project ---`／`Output Directory`／設定ファイルパス等のログは初期化後に出力され、
   従来どおりコンソールにもファイルにも残る。

### ヘルパー関数の切り出し
- `log_file_path(project_dir: &Path) -> PathBuf`：`project_dir.join("run.log")` を返すだけの
  純粋関数。単体テストする（命名・配置の固定を回帰防止）。
- ログ初期化本体（ファイルの追記オープン、subscriber 配線、`WorkerGuard` 返却）は
  `init_logging(project_dir) -> Result<WorkerGuard>` にまとめる。グローバル subscriber は
  1 プロセス 1 回しか init できず単体テストしにくいため、配線部分は実機・手動確認とし、
  ユニットテストは `log_file_path` に限定する。

### テスト
- 新規：`log_file_path` が `project_dir/run.log` を返すことを検証。
- 既存維持：`main.rs` の CLI／`resolve_config_path` 系テストはそのまま。

## 検証（受け入れ条件）

- `activate_required` が複数エンジンを並行に起動する（テストで経過時間により実証）。
- 1 エンジンの起動失敗時に即エラーを返す（既存 fail-fast テストが通る）。
- 実行後、`project_dir/run.log` が生成され、コンソールと同じ内容のログ（ローカル時刻付き）が含まれる。
- 2 回実行すると `run.log` に両方の実行ログが追記され、時刻と `--- Project: X ---` 行で区別できる。
- 既存テストがすべて通る。`cargo build` / `cargo test` が成功する。
