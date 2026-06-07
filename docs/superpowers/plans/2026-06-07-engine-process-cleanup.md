# エンジンプロセスの確実な後始末（Job Object 化） Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** `exe_path` 設定で自動起動したエンジンプロセス（VOICEVOX/AivisSpeech/XTTSのランチャーとその孫プロセス）を、通常のエラー終了では同期的・確実に、クラッシュ等の不測の終了ではOSのJob Object機構に任せて、必ず後始末されるようにする。

**Architecture:** Windows の Job Object（`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`）を、エンジンプロセス起動時に新規作成し、spawn したプロセスを割り当てる。`crates/s2v-engines/src/job.rs` に薄い RAII ラッパー `EngineJob` を実装し、`process.rs` の `EngineProcess`（`Child` + `EngineJob` を束ねる構造体）経由で `ensure_running`/`terminate_process` に組み込む。明示的な終了経路では `TerminateJobObject` を呼んで同期的にツリー全体を倒し、`Drop` でハンドルを閉じることで `KILL_ON_JOB_CLOSE` によるOS主導の自動後始末（クラッシュ・Ctrl+C等）も同時に実現する。

**Tech Stack:** Rust, `windows-sys` 0.61（Win32 JobObjects API）, tokio, 既存の `s2v-engines` クレート

参照spec: `docs/superpowers/specs/2026-06-07-engine-process-cleanup-design.md`

> **Note:** spec 4.2/6 は `#[cfg(windows)]` でのガードに言及しているが、`s2v-engines` クレートには現状 `cfg(windows)`/`cfg(target_os)` によるガードが一切なく（`std::os::windows::io::AsRawHandle` 等のWindows専用APIを既に無条件で使っている）、クレート全体が暗黙的にWindows専用という前提で書かれている。`job.rs` だけを `#[cfg(windows)]` でガードすると非Windowsターゲット向けの代替実装（スタブ）が別途必要になり、既存の方針からも今回のスコープからも外れるため、本プランでは既存の慣習に合わせてガードを追加しない（`Cargo.toml` の `[target.'cfg(windows)'.dependencies]` により `windows-sys` 自体は非Windows向けビルドでは取得されない）。

---

### Task 1: `windows-sys` 依存関係の追加

**Files:**
- Modify: `crates/s2v-engines/Cargo.toml`

- [ ] **Step 1: Cargo.toml に windows-sys を Windows 専用依存として追加する**

`crates/s2v-engines/Cargo.toml` の `[dependencies]` セクションの直後に以下を追記する:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_System_JobObjects",
    "Win32_System_Threading",
] }
```

- [ ] **Step 2: ビルドできることを確認する**

Run: `cargo build -p s2v-engines`
Expected: `Finished` で正常終了する（依存関係が解決されること。まだ `windows-sys` を参照するコードはないため、警告等は出ない）

- [ ] **Step 3: コミット**

```bash
git add crates/s2v-engines/Cargo.toml
git commit -m "build(engines): add windows-sys dependency for Job Object support"
```

---

### Task 2: `EngineJob`（Job Object の RAII ラッパー）を実装する

**Files:**
- Create: `crates/s2v-engines/src/job.rs`
- Modify: `crates/s2v-engines/src/lib.rs`

- [ ] **Step 1: テストモジュールを含む job.rs の骨格を書く（実装前なのでコンパイルは失敗する）**

`crates/s2v-engines/src/job.rs` を新規作成し、以下を書く:

```rust
//! Windows Job Object の RAII ラッパー。
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定した Job にエンジンプロセスを割り当てることで、
//! - 明示的に `terminate()` を呼べばランチャーと孫プロセスを含むツリー全体を即座に終了でき、
//! - 本体プロセスがクラッシュ・Ctrl+C 等で不意に終了し Job ハンドルが閉じられた場合も、
//!   OS が自動的にツリー全体を後始末してくれる。

use std::io;
use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

pub(crate) struct EngineJob {
    handle: HANDLE,
}

// SAFETY: HANDLE はカーネルオブジェクトへの不透明なポインタであり、
// 対応する Win32 API（AssignProcessToJobObject/TerminateJobObject 等）は
// どのスレッドから呼んでもよい。
unsafe impl Send for EngineJob {}
unsafe impl Sync for EngineJob {}

impl EngineJob {
    /// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定した無名 Job Object を作成する。
    pub(crate) fn new() -> io::Result<Self> {
        todo!()
    }

    /// 指定したプロセスをこの Job に割り当てる。
    /// 以後そのプロセスが起動する子プロセス（孫プロセス）も同じ Job に属する。
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        todo!()
    }

    /// Job 配下の全プロセス（ツリー全体）を即座に終了する。
    pub(crate) fn terminate(&self) -> io::Result<()> {
        todo!()
    }
}

impl Drop for EngineJob {
    fn drop(&mut self) {
        todo!()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spawn_long_running() -> Child {
        std::process::Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap()
    }

    #[test]
    fn terminate_kills_assigned_process() {
        let mut child = spawn_long_running();
        assert!(child.try_wait().unwrap().is_none(), "プロセスが起動していること");

        let job = EngineJob::new().unwrap();
        job.assign(&child).unwrap();
        job.terminate().unwrap();

        std::thread::sleep(Duration::from_millis(300));
        assert!(child.try_wait().unwrap().is_some(), "Job経由で終了していること");
    }

    #[test]
    fn dropping_job_terminates_assigned_process_via_kill_on_close() {
        let mut child = spawn_long_running();
        assert!(child.try_wait().unwrap().is_none(), "プロセスが起動していること");

        {
            let job = EngineJob::new().unwrap();
            job.assign(&child).unwrap();
            // ここで terminate() を呼ばずに job をドロップする（クラッシュ相当の状況を模す）
        }

        std::thread::sleep(Duration::from_millis(300));
        assert!(
            child.try_wait().unwrap().is_some(),
            "Jobハンドルのクローズで自動終了していること(KILL_ON_JOB_CLOSE)"
        );
    }
}
```

- [ ] **Step 2: テストを実行して失敗することを確認する**

Run: `cargo test -p s2v-engines job::`
Expected: コンパイルエラー（`todo!()` によるパニック、または `not yet implemented`）。`terminate_kills_assigned_process` が `not yet implemented` でパニックして FAIL する

- [ ] **Step 3: `EngineJob` を実装する**

`job.rs` の4つの `todo!()` を以下の実装に置き換える:

```rust
    pub(crate) fn new() -> io::Result<Self> {
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;

        let ok = unsafe {
            SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
        };
        if ok == 0 {
            let err = io::Error::last_os_error();
            unsafe { CloseHandle(handle) };
            return Err(err);
        }

        Ok(Self { handle })
    }

    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        let process_handle = child.as_raw_handle() as HANDLE;
        let ok = unsafe { AssignProcessToJobObject(self.handle, process_handle) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub(crate) fn terminate(&self) -> io::Result<()> {
        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
```

`impl Drop for EngineJob` の `todo!()` は以下に置き換える:

```rust
    fn drop(&mut self) {
        // ハンドルを閉じる。terminate() を呼ばずにここに到達した場合
        // （例: 本体プロセスのクラッシュで最後のハンドルとして閉じられる場合）でも、
        // KILL_ON_JOB_CLOSE によりOSがJob配下のプロセスを自動的に終了する。
        unsafe { CloseHandle(self.handle) };
    }
```

- [ ] **Step 4: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-engines job::`
Expected: `test job::tests::terminate_kills_assigned_process ... ok` と `test job::tests::dropping_job_terminates_assigned_process_via_kill_on_close ... ok` の2つが PASS する

- [ ] **Step 5: lib.rs にモジュール宣言を追加する**

`crates/s2v-engines/src/lib.rs` の1行目 `pub mod engine;` の前に以下を追加する:

```rust
mod job;
```

（`job` は内部実装なので非公開モジュールとする。`process` と同様の扱い）

- [ ] **Step 6: コミット**

```bash
git add crates/s2v-engines/src/job.rs crates/s2v-engines/src/lib.rs
git commit -m "feat(engines): add EngineJob wrapper around Windows Job Objects"
```

---

### Task 3: `process.rs` を `EngineJob` で後始末するように改修する

**Files:**
- Modify: `crates/s2v-engines/src/process.rs`

- [ ] **Step 1: `EngineProcess` 構造体を追加し、`use` 文を更新する**

`crates/s2v-engines/src/process.rs` の冒頭の `use` 文を以下に置き換える（`use tracing::info;` を `use tracing::{info, warn};` に変更し、`crate::job::EngineJob` の import を追加）:

```rust
use std::future::Future;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use tracing::{info, warn};

use crate::job::EngineJob;
```

`const POLL_RETRIES: usize = 30;` の直後に以下を追加する:

```rust
/// 自動起動したエンジンプロセスと、その後始末用に割り当てた Job Object をまとめて保持する。
///
/// `job` を Drop すると Job ハンドルが閉じられ、`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` により
/// OS が Job 配下の全プロセス（ランチャー＋孫プロセス）を自動的に終了する。
pub(crate) struct EngineProcess {
    child: Child,
    job: EngineJob,
}
```

- [ ] **Step 2: `ensure_running`/`terminate_process` のシグネチャと実装を更新する**

`pub(crate) async fn ensure_running` のシグネチャの `process: &Mutex<Option<Child>>` を `process: &Mutex<Option<EngineProcess>>` に変更する。

関数本体のうち、プロセス spawn から保持までの部分（元の以下のコード）

```rust
    info!("[{name}] 起動を確認できません。プロセスを起動します: {path}");
    let child = Command::new(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("{name}: プロセスの起動に失敗しました: {e}"))?;
    *process.lock().unwrap() = Some(child);
```

を以下に置き換える:

```rust
    let job = EngineJob::new()
        .map_err(|e| anyhow::anyhow!("{name}: Job Object の作成に失敗しました: {e}"))?;

    info!("[{name}] 起動を確認できません。プロセスを起動します: {path}");
    let child = Command::new(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("{name}: プロセスの起動に失敗しました: {e}"))?;

    if let Err(e) = job.assign(&child) {
        warn!(
            "[{name}] プロセスを Job Object に割り当てられませんでした(孫プロセスは終了対象外になる可能性があります): {e}"
        );
    }

    *process.lock().unwrap() = Some(EngineProcess { child, job });
```

`pub(crate) fn terminate_process` のシグネチャの `process: &Mutex<Option<Child>>` を `process: &Mutex<Option<EngineProcess>>` に変更し、関数本体を以下に置き換える:

```rust
pub(crate) fn terminate_process(name: &str, process: &Mutex<Option<EngineProcess>>) {
    let mut guard = process.lock().unwrap();
    if let Some(mut entry) = guard.take() {
        info!("[{name}] エンジンプロセスを停止します。");
        if let Err(e) = entry.job.terminate() {
            warn!("[{name}] Job Object 経由の終了に失敗しました: {e}");
        }
        let _ = entry.child.kill();
        let _ = entry.child.wait();
    }
}
```

- [ ] **Step 3: 既存テストの型注釈と構築コードを `EngineProcess` に合わせて更新する**

`#[cfg(test)] mod tests` 内の以下のテストの型注釈 `Mutex<Option<Child>>` を `Mutex<Option<EngineProcess>>` に置き換える（4箇所、関数名で特定する）:
- `ensure_running_does_not_spawn_when_already_alive`
- `ensure_running_errors_when_not_alive_and_no_exe_path`
- `ensure_running_errors_when_exe_path_does_not_exist`
- `ensure_running_spawns_process_and_waits_until_alive`
- `terminate_process_is_noop_when_nothing_was_spawned`

`terminate_process_kills_running_process_and_clears_handle` テストを以下の内容に丸ごと置き換える（直接 `Child` を包んでいた箇所を `EngineProcess` の構築に変更する）:

```rust
    #[tokio::test]
    async fn terminate_process_kills_running_process_and_clears_handle() {
        let child = std::process::Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();

        let job = EngineJob::new().unwrap();
        job.assign(&child).unwrap();

        let process: Mutex<Option<EngineProcess>> = Mutex::new(Some(EngineProcess { child, job }));
        terminate_process("test", &process);

        assert!(process.lock().unwrap().is_none(), "ハンドルが解放されていること");
    }
```

- [ ] **Step 4: テストを実行して全て通ることを確認する**

Run: `cargo test -p s2v-engines process::`
Expected: `process::tests` 配下の全テスト（`ensure_running_*` 5件、`terminate_process_*` 2件）が `ok` で PASS する

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-engines/src/process.rs
git commit -m "feat(engines): assign auto-started engine processes to a Job Object on spawn"
```

---

### Task 4: `HttpEngine` のフィールド型を `EngineProcess` に更新する

**Files:**
- Modify: `crates/s2v-engines/src/http_engine.rs:1-47`

- [ ] **Step 1: import とフィールド型を更新する**

`crates/s2v-engines/src/http_engine.rs:3` の `use std::process::Child;` を削除する。

`crates/s2v-engines/src/http_engine.rs:14` を以下に置き換える:

```rust
use crate::process::{ensure_running, terminate_process, EngineProcess};
```

`crates/s2v-engines/src/http_engine.rs:25` の `process: Mutex<Option<Child>>,` を以下に置き換える:

```rust
    process: Mutex<Option<EngineProcess>>,
```

`crates/s2v-engines/src/http_engine.rs:45` の `process: Mutex::new(None),` はそのままでよい（型推論で `Option<EngineProcess>` になる）。

- [ ] **Step 2: ビルド・テストを実行して通ることを確認する**

Run: `cargo test -p s2v-engines http_engine::`
Expected: コンパイルが通り、`http_engine::tests` 配下の全テストが `ok` で PASS する

- [ ] **Step 3: コミット**

```bash
git add crates/s2v-engines/src/http_engine.rs
git commit -m "refactor(engines): switch HttpEngine process tracking to EngineProcess"
```

---

### Task 5: `XttsEngine` のフィールド型を `EngineProcess` に更新する

**Files:**
- Modify: `crates/s2v-engines/src/xtts_engine.rs:1-44`

- [ ] **Step 1: import とフィールド型を更新する**

`crates/s2v-engines/src/xtts_engine.rs:3` の `use std::process::Child;` を削除する。

`crates/s2v-engines/src/xtts_engine.rs:14` を以下に置き換える:

```rust
use crate::process::{ensure_running, terminate_process, EngineProcess};
```

`crates/s2v-engines/src/xtts_engine.rs:22` の `process: Mutex<Option<Child>>,` を以下に置き換える:

```rust
    process: Mutex<Option<EngineProcess>>,
```

`crates/s2v-engines/src/xtts_engine.rs:42` の `process: Mutex::new(None),` はそのままでよい。

- [ ] **Step 2: ビルド・テストを実行して通ることを確認する**

Run: `cargo test -p s2v-engines xtts_engine::`
Expected: コンパイルが通り、`xtts_engine::tests` 配下の全テストが `ok` で PASS する

- [ ] **Step 3: コミット**

```bash
git add crates/s2v-engines/src/xtts_engine.rs
git commit -m "refactor(engines): switch XttsEngine process tracking to EngineProcess"
```

---

### Task 6: 孫プロセスも後始末されることを検証する統合テストを追加する

**Files:**
- Modify: `crates/s2v-engines/src/process.rs`

- [ ] **Step 1: 失敗するテストを書く**

`process.rs` の `#[cfg(test)] mod tests` 内、`write_marker_script` 関数の直後に以下のヘルパーとテストを追加する:

```rust
    /// ランチャーが孫プロセスを spawn し続けるダミースクリプト一式を書き出す。
    /// ランチャー自身は起動直後に `launcher_marker.txt` を作成し（is_alive 用の合図）、
    /// 孫プロセスは `grandchild_log.txt` に "alive" を1行ずつ追記し続ける。
    fn write_launcher_with_grandchild(dir: &std::path::Path) -> std::path::PathBuf {
        let grandchild = dir.join("grandchild.cmd");
        std::fs::write(
            &grandchild,
            "@echo off\r\n:loop\r\necho alive >> \"%~dp0grandchild_log.txt\"\r\nping -n 2 127.0.0.1 > nul\r\ngoto loop\r\n",
        )
        .unwrap();

        let launcher = dir.join("launcher.cmd");
        std::fs::write(
            &launcher,
            "@echo off\r\necho ready > \"%~dp0launcher_marker.txt\"\r\nstart \"\" /min cmd /c \"%~dp0grandchild.cmd\"\r\n:loop\r\nping -n 2 127.0.0.1 > nul\r\ngoto loop\r\n",
        )
        .unwrap();

        launcher
    }

    #[tokio::test]
    async fn terminate_process_kills_grandchild_processes_via_job_object() {
        let dir = tempfile::tempdir().unwrap();
        let launcher = write_launcher_with_grandchild(dir.path());
        let marker = dir.path().join("launcher_marker.txt");
        let log = dir.path().join("grandchild_log.txt");

        let process: Mutex<Option<EngineProcess>> = Mutex::new(None);
        let marker_for_check = marker.clone();
        ensure_running("test", launcher.to_str(), &process, move || {
            let marker = marker_for_check.clone();
            async move { marker.exists() }
        })
        .await
        .unwrap();

        // 孫プロセスがログに書き込み始めるまで待つ(最大15秒)
        let mut lines_before = 0usize;
        for _ in 0..30 {
            if let Ok(content) = std::fs::read_to_string(&log) {
                lines_before = content.lines().count();
                if lines_before > 0 {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert!(lines_before > 0, "孫プロセスが起動してログに書き込み始めていること");

        terminate_process("test", &process);

        // 孫プロセスが書き込みを止めた(=終了した)ことを確認する
        tokio::time::sleep(Duration::from_secs(3)).await;
        let lines_after = std::fs::read_to_string(&log).unwrap().lines().count();
        assert_eq!(
            lines_after, lines_before,
            "Job Object経由でランチャーだけでなく孫プロセスも終了していること"
        );
    }
```

`process.rs` 冒頭のテスト用 `use` 文に `Duration` が無ければ追加する（`use std::time::Duration;` — 既に `process.rs` 本体側で `use std::time::Duration;` を import 済みのため、`use super::*;` 経由でテストモジュールからもアクセス可能。追加の import は不要）。

- [ ] **Step 2: テストを実行して通ることを確認する**

Run: `cargo test -p s2v-engines process::tests::terminate_process_kills_grandchild_processes_via_job_object -- --nocapture`
Expected: `test process::tests::terminate_process_kills_grandchild_processes_via_job_object ... ok` で PASS する（実行に10〜20秒程度かかる）

- [ ] **Step 3: クレート全体のテストを実行して既存テストに影響がないことを確認する**

Run: `cargo test -p s2v-engines`
Expected: 全テストが `ok` で PASS する（`test result: ok. ... 0 failed`）

- [ ] **Step 4: コミット**

```bash
git add crates/s2v-engines/src/process.rs
git commit -m "test(engines): verify Job Object termination kills grandchild processes too"
```

---

### Task 7: ワークスペース全体のビルド・テストで最終確認する

**Files:** (変更なし、確認のみ)

- [ ] **Step 1: ワークスペース全体をビルドする**

Run: `cargo build --release`
Expected: `Finished \`release\` profile [optimized] target(s)` で正常終了する

- [ ] **Step 2: ワークスペース全体のテストを実行する**

Run: `cargo test --workspace`
Expected: 全クレートのテストが `ok` で PASS する

- [ ] **Step 3: 実行ファイルが新しいバイナリに更新されたことを確認する**

Run: `Get-Item "target/release/script2voice.exe" | Select-Object LastWriteTime`
（PowerShellツールで実行）
Expected: ビルド実行時刻に更新されている
