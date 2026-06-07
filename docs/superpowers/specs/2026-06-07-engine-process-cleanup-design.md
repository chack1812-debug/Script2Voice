# エンジンプロセスの確実な後始末（Job Object 化）設計書

**日付:** 2026-06-07
**対象プロジェクト:** D:\UDS\Script2Voice-Rust版
**関連:** s2v-9vr（エンジン自動起動・自動停止）、`crates/s2v-engines/src/process.rs`

---

## 1. 背景・課題

`exe_path` 設定によりエンジン（VOICEVOX/AivisSpeech/XTTS）を自動起動した場合、現状の `terminate_process`（`process.rs`）は `Child::kill()`（Windowsの `TerminateProcess`）で**直接の子プロセスのみ**を終了させている。

しかし VOICEVOX の `run.exe` や AivisSpeech の `run.exe` は内部に `engine_internal` ディレクトリや Python ランタイム一式を含んでおり、起動時に**エンジン本体（孫プロセス）を別途 spawn している可能性が高い**構成になっている。この場合、ランチャー(`run.exe`)だけが kill され、実際にポートを掴んでいるエンジン本体プロセスが孤立して残り続ける。

また、現状は「`run_pipeline` が `Result::Err` を返すケース」では `main()` の `engine_manager.shutdown_all()` が確実に呼ばれるが、**Ctrl+C・パニック・タスクマネージャ等からの強制終了**など、後始末コードが一切実行されないケースには無防備である。

## 2. 要件

1. **通常のエラー等で実行不能になった場合**: ランチャーが起動した孫プロセスを含め、エンジンプロセスツリー全体を**確実に終了させてから**本体プログラムを終了する（明示的・同期的な後始末）
2. **クラッシュ等の不測の終了の場合**: 本体プログラム側のコードが一切実行されなくても、**OSの仕組みに依存して**エンジンプロセスツリーが自動的に後始末される

## 3. 採用方式: Windows Job Object

Windows の Job Object 機能（`CreateJobObjectW` / `AssignProcessToJobObject` / `SetInformationJobObject` / `TerminateJobObject`）を使う。

- エンジンプロセスを spawn する際、`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定した Job Object を作成し、その子プロセスを Job に割り当てる
- Job に割り当てられたプロセスが新たに起動する子プロセス（孫プロセス）も、デフォルトで同じ Job に属する
- **明示的終了**: `TerminateJobObject(job, exit_code)` を呼べば、Job 配下の全プロセスが同期的に即時終了する。`child.wait()` で完了を待ってから戻ることで「確実に終了させてから本体の終了」を満たす
- **自動終了（OS 任せ）**: `KILL_ON_JOB_CLOSE` を設定した Job は、最後のハンドルが閉じられた時点（＝本体プロセスが終了した時点。クラッシュ・強制終了でも OS がプロセス終了時にハンドルを閉じる）で配下の全プロセスを自動的に終了する。本体側のコードは一切不要で「OS の処理に依存する」を満たす

この方式は1つの仕組みで要件1・2の両方を満たせる。

### 検討した代替案

- **`taskkill /T /F /PID <pid>` を呼ぶ**: 実装は簡単だが、明示的に呼び出すコードが実行されるケース（要件1）にしか効かず、要件2（クラッシュ等）には無力。不採用。

## 4. 実装範囲

### 4.1 依存関係追加
- `crates/s2v-engines/Cargo.toml` に `windows-sys`（バージョンは依存ツリーに既存の 0.61系に合わせる）を追加し、`Win32_Foundation` / `Win32_System_JobObjects` / `Win32_System_Threading` 相当の機能を有効化する

### 4.2 新規モジュール: `crates/s2v-engines/src/job.rs`
Job Object の FFI を安全にラップする RAII 構造体（仮称 `EngineJob`）を実装する。

- `EngineJob::new() -> io::Result<Self>`: Job Object を作成し `KILL_ON_JOB_CLOSE` を設定
- `EngineJob::assign(&self, child: &Child) -> io::Result<()>`: 指定プロセスを Job に割り当てる
  - `std::process::Command::spawn()` は生成直後のスレッドハンドルを返さないため `CREATE_SUSPENDED` + `ResumeThread` による厳密な競合排除はできない。`spawn()` 直後、可能な限り早いタイミングで `AssignProcessToJobObject` を呼ぶ（マイクロ秒オーダーの競合窓は残るが、エンジン起動処理（DLL読み込み・サーバー初期化）の所要時間と比べて無視できるレベルであり、実用上問題ない）
- `EngineJob::terminate(&self) -> io::Result<()>`: `TerminateJobObject` で Job 配下を即時終了する
- `Drop` でハンドルを閉じる（＝明示的に `terminate` しなかった場合、本体終了時に OS が `KILL_ON_JOB_CLOSE` により配下を巻き込み終了する）
- Windows 専用 API のため `#[cfg(windows)]` でガードする

### 4.3 `process.rs` の改修
- `Mutex<Option<Child>>` を `Mutex<Option<EngineProcess>>`（`Child` と `EngineJob` をまとめた構造体）に変更する
- `ensure_running`: プロセス spawn 直後に `EngineJob` を作成し、spawn したプロセスを割り当てて `EngineProcess` として保持する
- `terminate_process`: `EngineJob::terminate()` で Job 配下を同期的に終了 → `child.wait()` で完了を待つ、という流れに変更する

### 4.4 呼び出し側（`http_engine.rs` / `xtts_engine.rs`）
- `process: Mutex<Option<Child>>` 等の型を `process.rs` で定義する新しい型に合わせて変更する。インターフェース（`ensure_running`/`terminate_process` の呼び出し方）自体は変えない

## 5. テスト方針

- `job.rs`: Job Object の作成・プロセス割り当て・`TerminateJobObject` による終了が機能することを確認する単体テスト
- `process.rs`: 既存の `ensure_running_spawns_process_and_waits_until_alive` / `terminate_process_kills_running_process_and_clears_handle` 等のテストを、孫プロセスを spawn するダミースクリプト（cmd バッチで子プロセスをさらに起動するもの）に拡張し、以下を検証する
  - 明示的な `terminate_process` 呼び出しで、ランチャー・孫プロセスの両方が終了すること（同期的に確認できること）
  - Job ハンドルを明示的に終了させずに（＝クラッシュを模した状況で）ドロップした場合に、`KILL_ON_JOB_CLOSE` により配下プロセスが終了すること

## 6. 影響範囲・互換性

- 公開インターフェース（`Engine` トレイト、`EngineManager`、`HttpEngine`/`XttsEngine` のコンストラクタ）に変更はない。内部実装のみの変更
- Windows 専用 API を使うため `#[cfg(windows)]` でガードし、本プロジェクトは Windows 専用であることを前提とする
