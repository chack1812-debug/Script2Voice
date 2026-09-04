# エンジンプロセスのマルチプロセス共有 実装計画

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Script2Voice を複数プロセス同時実行しても、先に終了したプロセスが他プロセスの使用中の
共有エンジンを終了させてしまわないようにする。

**Architecture:** 無名 Job Object をエンジンごとの名前付き Job Object
（`Local\Script2Voice_Engine_<key>`）に変更し、そのエンジンを使う全プロセスがハンドルを保持する。
`terminate_process` からは明示的な `TerminateJobObject` / `Child::kill` を廃止し、最後のハンドルが
閉じたときに `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` で OS がエンジンツリーを終了させる。あわせて
「起動確認 → spawn → 起動完了待ち」の臨界区間をロックファイル（`FILE_SHARE_NONE` 排他オープン）で
プロセス間排他し、二重 spawn を防ぐ。

**Tech Stack:** Rust / tokio / windows-sys 0.61（Win32_Foundation, Win32_System_JobObjects）/
std::os::windows::fs::OpenOptionsExt

**Spec:** `docs/superpowers/specs/2026-09-04-engine-multiprocess-sharing-design.md`

**Bead:** `s2v-zm9`（P0 / bug）

## Global Constraints

- 対象プラットフォームは Windows のみ（`crates/s2v-engines` は既に `job.rs` が Windows 専用）。
- **新規クレート依存を追加しない。** ロックは `std::os::windows::fs::OpenOptionsExt` のみで実装する
  （`fs2` / `fs4` を追加しない）。`windows-sys` の feature も現状のまま
  （`Win32_Foundation`, `Win32_Security`, `Win32_System_JobObjects`, `Win32_System_Threading`）で足りる。
- Job Object 名は `Local\Script2Voice_Engine_<key>`、ロックファイルは
  `%TEMP%\script2voice_engine_<key>.lock`。`<key>` は `engine_resource_key(name, url)` が返す文字列。
- 名前空間は `Local\`（同一ユーザーセッション内の複数起動のみを想定）。
- **テストは Job 名・ロックキーをテストごとに一意にする。** Rust のテストは同一プロセス内のスレッドで
  並列実行されるため、名前が固定だと別テスト同士が同じ Job / ロックを共有して偽陽性・偽陰性になる。
- 既存のログ文言・コメントは日本語。新規のログ・doc コメントも日本語で書く。
- スコープ外: 合成失敗行のリトライ（bead `s2v-cgj`）、OpenJTalk 辞書ロック（voicevox_engine #1347）。
- 各タスク末尾でコミットする。**push はしない。**

---

## File Structure

| ファイル | 責務 |
| --- | --- |
| `crates/s2v-engines/src/process.rs` | エンジンの起動確認・spawn・後始末。`engine_resource_key` もここに置く（Job 名とロック名の両方の材料であり、`ensure_running` が唯一の利用者のため） |
| `crates/s2v-engines/src/job.rs` | 名前付き Job Object の RAII ラッパー。`open_or_create` / `assign` のみ |
| `crates/s2v-engines/src/lock.rs` | **新規。** 起動処理のプロセス間排他ロック（ロックファイルの RAII ガード） |
| `crates/s2v-engines/src/lib.rs` | `mod lock;` の追加 |
| `crates/s2v-engines/src/http_engine.rs` | `ensure_running` に key を渡す |
| `crates/s2v-engines/src/xtts_engine.rs` | 同上 |

---

## Task 1: エンジン資源キーの導出

Job 名とロックファイル名の材料になる、エンジンごとに一意なキーを作る純関数を追加する。
同じエンジン種別でもポートが違えば別インスタンスなので、ポートまで含める。

**Files:**
- Modify: `crates/s2v-engines/src/process.rs`（末尾の `#[cfg(test)] mod tests` の直前に関数を追加、
  テストは `mod tests` 内に追加）

**Interfaces:**
- Consumes: なし
- Produces: `pub(crate) fn engine_resource_key(name: &str, url: &str) -> String`

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-engines/src/process.rs` の `mod tests` 内の末尾に追加する。

```rust
    #[test]
    fn engine_resource_key_uses_port_from_url() {
        assert_eq!(engine_resource_key("voicevox", "http://127.0.0.1:50021"), "voicevox_50021");
        assert_eq!(engine_resource_key("aivis", "http://127.0.0.1:10101/"), "aivis_10101");
        assert_eq!(engine_resource_key("xtts", "http://127.0.0.1:8020/api"), "xtts_8020");
    }

    #[test]
    fn engine_resource_key_differs_per_port_for_same_engine() {
        assert_ne!(
            engine_resource_key("voicevox", "http://127.0.0.1:50021"),
            engine_resource_key("voicevox", "http://127.0.0.1:50022"),
        );
    }

    #[test]
    fn engine_resource_key_falls_back_to_sanitized_url_without_port() {
        // ポートを取り出せない URL は、英数字以外を '_' に潰した URL 全体を使う。
        // Job 名・ファイル名に使うため、'/' や ':' が残っていてはいけない。
        let key = engine_resource_key("voicevox", "http://localhost");
        assert_eq!(key, "voicevox_http___localhost");
        assert!(key.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'));
    }
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p s2v-engines engine_resource_key`
Expected: コンパイルエラー `cannot find function 'engine_resource_key' in this scope`

- [ ] **Step 3: 最小限の実装を書く**

`crates/s2v-engines/src/process.rs` の `terminate_process` の直後、`#[cfg(test)] mod tests` の直前に追加する。

```rust
/// エンジンごとに一意な資源名（Job Object 名・ロックファイル名）の材料を作る。
///
/// 同じエンジン種別でもポートが違えば別インスタンスなので、`url` のポート番号まで含める。
/// ポートを取り出せない `url` の場合は、`url` 全体の英数字以外を `_` に潰したものを使う
/// （Job 名・ファイル名として使えるようにするため）。
pub(crate) fn engine_resource_key(name: &str, url: &str) -> String {
    let port = url.rsplit_once(':').and_then(|(_, rest)| {
        let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
        (!digits.is_empty()).then_some(digits)
    });
    let suffix = port.unwrap_or_else(|| sanitize_for_resource_name(url));
    format!("{}_{}", sanitize_for_resource_name(name), suffix)
}

fn sanitize_for_resource_name(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect()
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-engines engine_resource_key`
Expected: 3 tests PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-engines/src/process.rs
git commit -m "feat(engines): エンジン資源キーの導出関数を追加する"
```

---

## Task 2: 名前付き Job Object によるエンジン生存期間の共有

今回のバグの本丸。無名 Job を名前付きにし、そのエンジンを使う全プロセスがハンドルを保持するようにする。
`terminate_process` からは明示的な終了処理を廃止する。

この3点（Job の名前付き化・`ensure_running` での常時保持・`terminate_process` の明示終了廃止）は
分割するとコンパイルが通らないか、バグが半分残るため、1タスクにまとめる。

**Files:**
- Modify: `crates/s2v-engines/src/job.rs`
- Modify: `crates/s2v-engines/src/process.rs`
- Modify: `crates/s2v-engines/src/http_engine.rs:120`
- Modify: `crates/s2v-engines/src/xtts_engine.rs:78`

**Interfaces:**
- Consumes: `engine_resource_key(name, url) -> String`（Task 1）
- Produces:
  - `EngineJob::open_or_create(name: &str) -> std::io::Result<EngineJob>`（`EngineJob::new()` を置き換え）
  - `EngineJob::assign(&self, child: &std::process::Child) -> std::io::Result<()>`（変更なし）
  - `EngineJob::terminate` は**削除**する
  - `EngineProcess { child: Option<std::process::Child>, job: EngineJob }`（`child` が `Option` になる）
  - `ensure_running(name: &str, key: &str, exe_path: Option<&str>, args: &[String], timeout: Duration, process: &Mutex<Option<EngineProcess>>, is_alive: F) -> anyhow::Result<()>`
    （第2引数に `key` が挿入される）

- [ ] **Step 1: 失敗するテストを書く（job.rs — 共有セマンティクスの回帰テスト）**

`crates/s2v-engines/src/job.rs` の `mod tests` に、一意名ヘルパーと新規テストを追加する。

```rust
    /// Rust のテストは同一プロセス内のスレッドで並列実行されるため、
    /// Job 名が固定だと別テスト同士が同じ Job を共有してしまう。テストごとに一意にする。
    fn unique_job_name(tag: &str) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        format!("Local\\s2v_test_{}_{}_{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst))
    }

    /// 今回のバグ(先に終了したプロセスが共有エンジンを殺す)に対応する回帰テスト。
    /// 同名 Job のハンドルを2つ開くことで、2プロセスが共有している状況を同一プロセス内で再現する。
    #[test]
    fn assigned_process_survives_until_last_handle_is_dropped() {
        let name = unique_job_name("shared");
        let mut child = spawn_long_running();

        let first = EngineJob::open_or_create(&name).unwrap();
        first.assign(&child).unwrap();
        let second = EngineJob::open_or_create(&name).unwrap();

        drop(first);
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            child.try_wait().unwrap().is_none(),
            "他プロセス相当のハンドルが残っている間はエンジンが生き続けること"
        );

        drop(second);
        std::thread::sleep(Duration::from_millis(500));
        assert!(
            child.try_wait().unwrap().is_some(),
            "最後のハンドルが閉じた時点でエンジンが終了すること"
        );
    }
```

同じ `mod tests` 内で、既存テストを次のように置き換える。

- `terminate_kills_assigned_process` は **削除**する（`terminate()` を廃止するため）。
- `dropping_job_terminates_assigned_process_via_kill_on_close` の
  `let job = EngineJob::new().unwrap();` を
  `let job = EngineJob::open_or_create(&unique_job_name("drop")).unwrap();` に変更する。

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p s2v-engines --lib job::`
Expected: コンパイルエラー `no function or associated item named 'open_or_create' found`

- [ ] **Step 3: job.rs を名前付き Job に変更する**

`crates/s2v-engines/src/job.rs` の import を差し替える。

```rust
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE};
```

`EngineJob::new` を次で置き換える（`impl EngineJob` の中）。

```rust
    /// 名前付き Job Object を開く（存在しなければ作成する）。
    ///
    /// 同じ名前で開いたハンドルはプロセスを跨いで同じ Job を指す。エンジンを共有する
    /// 全プロセスがハンドルを保持することで、「そのエンジンを使っている最後のプロセスが
    /// 終了した瞬間に、OS が `KILL_ON_JOB_CLOSE` でエンジンツリーを終了させる」という意味になる。
    pub(crate) fn open_or_create(name: &str) -> io::Result<Self> {
        let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
        let handle = unsafe { CreateJobObjectW(std::ptr::null(), wide.as_ptr()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }

        // 既存の Job を開いた場合は作成者が設定済みなので触らない。
        // 新規作成時だけ KILL_ON_JOB_CLOSE を設定する。
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            return Ok(Self { handle });
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
```

`terminate()` メソッドを**削除**し、import から `TerminateJobObject` を外す。
ファイル冒頭の doc コメントを実態に合わせて次に差し替える。

```rust
//! Windows Job Object の RAII ラッパー。
//!
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` を設定した名前付き Job にエンジンプロセスを割り当て、
//! そのエンジンを使う全プロセスが同名 Job のハンドルを保持する。
//! ハンドルが1つでも残っている間はエンジンが生き続け、最後の1つが閉じられた瞬間に
//! OS がエンジンツリー全体を終了させる。正常終了・クラッシュ・強制終了のいずれでも動く。
```

- [ ] **Step 4: process.rs を共有前提に書き換える**

`crates/s2v-engines/src/process.rs` の `EngineProcess` を次に変更する。

```rust
/// 使用中のエンジンへの参照を保持する。
///
/// `job` は名前付き Job Object のハンドル。自分がエンジンを spawn したかどうかに関わらず、
/// エンジンを使っている間ずっと保持する。Drop するとハンドルが閉じ、自分が最後の保持者なら
/// `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` により OS がエンジンツリーを終了させる。
///
/// `child` は自分が spawn した場合のみ `Some`。既に起動していたエンジンを使う場合は `None`。
pub(crate) struct EngineProcess {
    child: Option<Child>,
    job: EngineJob,
}
```

`ensure_running` のシグネチャと先頭部分を次に変更する。

```rust
pub(crate) async fn ensure_running<F, Fut>(
    name: &str,
    key: &str,
    exe_path: Option<&str>,
    args: &[String],
    timeout: Duration,
    process: &Mutex<Option<EngineProcess>>,
    is_alive: F,
) -> anyhow::Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    let job = EngineJob::open_or_create(&format!("Local\\Script2Voice_Engine_{key}"))
        .map_err(|e| anyhow::anyhow!("{name}: Job Object の作成に失敗しました: {e}"))?;

    if is_alive().await {
        info!("[{name}] 既に起動しています。");
        // 自分が起動していなくても Job ハンドルは保持する。
        // これを保持しないと、エンジンを起動したプロセスが先に終了した時点で
        // 使用中のエンジンが終了してしまう。
        *process.lock().unwrap() = Some(EngineProcess { child: None, job });
        return Ok(());
    }

    let path = exe_path.ok_or_else(|| {
        anyhow::anyhow!("{name}: サーバーに接続できず、exe_path も未設定のため起動できません")
    })?;
```

続く `let job = EngineJob::new()...` の4行（`job` の生成）は**削除**する（上で生成済みのため）。
`*process.lock().unwrap() = Some(EngineProcess { child, job });` を
`*process.lock().unwrap() = Some(EngineProcess { child: Some(child), job });` に変更する。

`terminate_process` を次で置き換える。

```rust
/// エンジンへの参照（名前付き Job のハンドル）を解放する。
///
/// 明示的にエンジンを終了させることはしない。自分が最後の保持者であれば、ハンドルが閉じた
/// 時点で OS が `KILL_ON_JOB_CLOSE` によりエンジンツリーを終了させる。他の Script2Voice
/// プロセスがまだ同じエンジンを使っている場合はエンジンが生き残る。
pub(crate) fn terminate_process(name: &str, process: &Mutex<Option<EngineProcess>>) {
    let mut guard = process.lock().unwrap();
    if let Some(entry) = guard.take() {
        info!("[{name}] エンジンへの参照を解放します（最後の利用者ならエンジンも終了します）。");
        drop(entry);
    }
}
```

- [ ] **Step 5: process.rs の既存テストを新シグネチャに合わせる**

`mod tests` の先頭（`write_marker_script` の直前）に一意キーのヘルパーを追加する。

```rust
    /// Job 名・ロックキーはテストごとに一意にする（テストは同一プロセス内で並列実行されるため）。
    fn unique_key(tag: &str) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        format!("test_{}_{}_{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst))
    }
```

`ensure_running` の呼び出し6箇所に、第2引数として一意キーを挿入する。
挿入するタグは呼び出しごとに次のとおり（重複させないこと）。

| 現在の行 | テスト | 挿入する第2引数 |
| --- | --- | --- |
| `process.rs:143` | `terminate_process_kills_grandchild_processes_via_job_object` | `&unique_key("grandchild")` |
| `process.rs:180` | `ensure_running_does_not_spawn_when_already_alive` | `&unique_key("alive")` |
| `process.rs:195` | `ensure_running_errors_when_not_alive_and_no_exe_path` | `&unique_key("noexe")` |
| `process.rs:204` | `ensure_running_errors_when_exe_path_does_not_exist` | `&unique_key("missing")` |
| `process.rs:225` | `ensure_running_spawns_path_resolved_command_with_args` | `&unique_key("pathcmd")` |
| `process.rs:246` | `ensure_running_spawns_process_and_waits_until_alive` | `&unique_key("spawnwait")` |

例（`process.rs:180` の場合）:

```rust
        ensure_running("test", &unique_key("alive"), None, &[], Duration::from_secs(30), &process, move || {
```

さらに次の3点を修正する。

1. `ensure_running_does_not_spawn_when_already_alive` の最後の assert を、新しい意味論に合わせる。

```rust
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let guard = process.lock().unwrap();
        let entry = guard.as_ref().expect("既存エンジンを使う場合も Job ハンドルを保持すること");
        assert!(entry.child.is_none(), "自分では spawn していないこと");
```

2. `terminate_process_kills_running_process_and_clears_handle`（`process.rs:268` 付近）の
   `EngineJob::new()` と `EngineProcess { child, job }` を差し替える。

```rust
        let job = EngineJob::open_or_create(&format!("Local\\Script2Voice_Engine_{}", unique_key("clear"))).unwrap();
        job.assign(&child).unwrap();

        let process: Mutex<Option<EngineProcess>> = Mutex::new(Some(EngineProcess { child: Some(child), job }));
        terminate_process("test", &process);

        assert!(process.lock().unwrap().is_none(), "ハンドルが解放されていること");
```

3. `terminate_process_kills_grandchild_processes_via_job_object` は名前と doc を実態に合わせて変更する
   （検証している機構が `TerminateJobObject` からハンドルクローズに変わったため）。アサーションは変更しない。

```rust
    /// 最後の参照が解放されたとき、ランチャーだけでなく孫プロセスまで終了することを確認する。
    /// VOICEVOX / AivisSpeech の run.exe はエンジン本体を孫プロセスとして起動するため、
    /// Job Object を使わないと孫プロセスが残ってしまう。
    #[tokio::test]
    async fn releasing_last_reference_kills_grandchild_processes_via_job_object() {
```

- [ ] **Step 6: 呼び出し元（http_engine / xtts_engine）に key を渡す**

`crates/s2v-engines/src/http_engine.rs` の import に `engine_resource_key` を追加する。

```rust
use crate::process::{engine_resource_key, ensure_running, terminate_process, EngineProcess, DEFAULT_STARTUP_TIMEOUT};
```

`activate` の `ensure_running` 呼び出し（120行目）を次に置き換える。

```rust
        ensure_running(
            &self.name,
            &engine_resource_key(&self.name, &self.url),
            self.exe_path.as_deref(),
            &self.args,
            self.startup_timeout,
            &self.process,
            || self.is_alive(),
        )
        .await?;
```

`crates/s2v-engines/src/xtts_engine.rs` にも同じ2点（import 追加・78行目の置き換え）を行う。

- [ ] **Step 7: テストが通ることを確認する**

Run: `cargo test -p s2v-engines`
Expected: 全 PASS。特に
`assigned_process_survives_until_last_handle_is_dropped` と
`releasing_last_reference_kills_grandchild_processes_via_job_object` が PASS すること。

- [ ] **Step 8: コミット**

```bash
git add crates/s2v-engines/src/job.rs crates/s2v-engines/src/process.rs crates/s2v-engines/src/http_engine.rs crates/s2v-engines/src/xtts_engine.rs
git commit -m "fix(engines): 名前付きJob Objectで共有エンジンの生存期間をプロセス間で共有する"
```

---

## Task 3: 起動処理のプロセス間排他ロック

ロックファイルの RAII ガードを新規モジュールとして追加する。この時点ではまだ `ensure_running` から
使わないため、`cargo build` で `dead_code` 警告が出るが Task 4 で解消する。

**Files:**
- Create: `crates/s2v-engines/src/lock.rs`
- Modify: `crates/s2v-engines/src/lib.rs`

**Interfaces:**
- Consumes: なし
- Produces:
  - `pub(crate) struct EngineStartupLock`（RAII ガード。drop でロック解放）
  - `pub(crate) fn lock_path(key: &str) -> std::path::PathBuf`
  - `pub(crate) fn try_acquire(key: &str) -> std::io::Result<Option<EngineStartupLock>>`
  - `pub(crate) async fn acquire(key: &str, timeout: Duration) -> Option<EngineStartupLock>`

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-engines/src/lock.rs` を新規作成し、まずテストだけを書く。

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn unique_key(tag: &str) -> String {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static SEQ: AtomicUsize = AtomicUsize::new(0);
        format!("locktest_{}_{}_{}", tag, std::process::id(), SEQ.fetch_add(1, Ordering::SeqCst))
    }

    #[test]
    fn second_acquire_fails_while_first_is_held() {
        let key = unique_key("held");
        let first = try_acquire(&key).unwrap();
        assert!(first.is_some(), "1回目は取得できること");
        assert!(try_acquire(&key).unwrap().is_none(), "保持中は取得できないこと");
    }

    #[test]
    fn lock_is_released_when_guard_is_dropped() {
        // プロセスがクラッシュした場合も OS が同じようにハンドルを閉じるため、
        // このテストは「クラッシュ時にロックが残らない」ことの代理検証になる。
        let key = unique_key("release");
        let first = try_acquire(&key).unwrap().unwrap();
        drop(first);
        assert!(try_acquire(&key).unwrap().is_some(), "drop で解放されること");
    }

    #[tokio::test]
    async fn acquire_gives_up_after_timeout() {
        let key = unique_key("timeout");
        let _held = try_acquire(&key).unwrap().unwrap();

        let start = std::time::Instant::now();
        assert!(acquire(&key, Duration::from_millis(300)).await.is_none());
        assert!(start.elapsed() >= Duration::from_millis(300), "タイムアウトまで待つこと");
    }

    #[tokio::test]
    async fn acquire_succeeds_once_the_holder_releases() {
        let key = unique_key("handoff");
        let held = try_acquire(&key).unwrap().unwrap();

        let key_for_task = key.clone();
        let waiter = tokio::spawn(async move { acquire(&key_for_task, Duration::from_secs(5)).await.is_some() });

        tokio::time::sleep(Duration::from_millis(300)).await;
        drop(held);

        assert!(waiter.await.unwrap(), "保持者が解放したら取得できること");
    }
}
```

- [ ] **Step 2: テストが失敗することを確認する**

`crates/s2v-engines/src/lib.rs` の `mod job;` の直後に `mod lock;` を追加してから実行する。

Run: `cargo test -p s2v-engines --lib lock::`
Expected: コンパイルエラー `cannot find function 'try_acquire' in this scope`

- [ ] **Step 3: 実装を書く**

`crates/s2v-engines/src/lock.rs` のテストモジュールの**上**に追加する。

```rust
//! エンジン起動処理のプロセス間排他ロック。
//!
//! Windows の named mutex はスレッドアフィニティ（`ReleaseMutex` は取得したスレッドから
//! 呼ぶ必要がある）を持つ。一方この臨界区間は `.await` をまたぎ、tokio のマルチスレッド
//! ランタイムでは `.await` の前後でワーカースレッドが移動しうるため named mutex は使えない。
//!
//! 代わりにロックファイルを `FILE_SHARE_NONE` で排他オープンする。スレッドアフィニティがなく、
//! プロセスが落ちれば OS がハンドルを閉じるためロックは自動的に解放される。

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::OpenOptionsExt;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use tracing::warn;

/// ロック取得のリトライ間隔。
const RETRY_INTERVAL: Duration = Duration::from_millis(100);

/// `FILE_SHARE_NONE`。他プロセスが開いている間は共有違反で失敗する。
const FILE_SHARE_NONE: u32 = 0;

/// `ERROR_SHARING_VIOLATION`。他プロセスがロックを保持している状態。
const ERROR_SHARING_VIOLATION: i32 = 32;

/// 保持している間だけロックを持つ RAII ガード。drop するとハンドルが閉じてロックが解放される。
pub(crate) struct EngineStartupLock {
    _file: File,
}

/// ロックファイルのパス。Job Object 名と同じ `key` で揃える。
pub(crate) fn lock_path(key: &str) -> PathBuf {
    std::env::temp_dir().join(format!("script2voice_engine_{key}.lock"))
}

/// ロックの取得を1回だけ試みる。他プロセスが保持していれば `Ok(None)`。
pub(crate) fn try_acquire(key: &str) -> io::Result<Option<EngineStartupLock>> {
    match OpenOptions::new()
        .create(true)
        .write(true)
        .share_mode(FILE_SHARE_NONE)
        .open(lock_path(key))
    {
        Ok(file) => Ok(Some(EngineStartupLock { _file: file })),
        Err(e) if e.raw_os_error() == Some(ERROR_SHARING_VIOLATION) => Ok(None),
        Err(e) => Err(e),
    }
}

/// `timeout` まで待ってロックを取得する。取れなければ `None`。
///
/// ブロッキングせず `tokio::time::sleep` でリトライするため、`.await` をまたいで保持してよい。
pub(crate) async fn acquire(key: &str, timeout: Duration) -> Option<EngineStartupLock> {
    let deadline = Instant::now() + timeout;
    loop {
        match try_acquire(key) {
            Ok(Some(lock)) => return Some(lock),
            Ok(None) => {}
            Err(e) => {
                // ロックファイルを開けない環境（権限等）では排他をあきらめて続行する。
                warn!("起動ロック {} を開けませんでした: {e}", lock_path(key).display());
                return None;
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        tokio::time::sleep(RETRY_INTERVAL).await;
    }
}
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-engines --lib lock::`
Expected: 4 tests PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-engines/src/lock.rs crates/s2v-engines/src/lib.rs
git commit -m "feat(engines): エンジン起動処理のプロセス間排他ロックを追加する"
```

---

## Task 4: 起動処理にロックを組み込む（二重 spawn の防止）

`ensure_running` の本体全体をロックで覆い、「P1 が spawn した直後、まだ `/version` が応答しない間に
P2 が来て二重に spawn する」のを防ぐ。

**Files:**
- Modify: `crates/s2v-engines/src/process.rs`

**Interfaces:**
- Consumes: `crate::lock::{acquire, try_acquire, EngineStartupLock}`（Task 3）、
  `ensure_running(name, key, exe_path, args, timeout, process, is_alive)`（Task 2）
- Produces: `ensure_running` の外部シグネチャは変わらない（挙動のみ変わる）

- [ ] **Step 1: 失敗するテストを書く**

`crates/s2v-engines/src/process.rs` の `mod tests` に、カウント用スクリプトのヘルパーと2つのテストを追加する。

```rust
    /// 起動されるたびに `count.txt` へ1行追記するダミーエンジン。
    /// 二重 spawn が起きれば行数が2以上になる。
    fn write_counting_script(dir: &std::path::Path) -> std::path::PathBuf {
        let script = dir.join("counting_engine.cmd");
        std::fs::write(&script, "@echo off\r\necho spawned >> \"%~dp0count.txt\"\r\n").unwrap();
        script
    }

    /// 2プロセスがほぼ同時に起動を試みても、排他により engine の spawn は1回だけであること。
    #[tokio::test]
    async fn concurrent_ensure_running_spawns_engine_only_once() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_counting_script(dir.path());
        let count = dir.path().join("count.txt");
        let key = unique_key("dup");

        let p1: Mutex<Option<EngineProcess>> = Mutex::new(None);
        let p2: Mutex<Option<EngineProcess>> = Mutex::new(None);
        let c1 = count.clone();
        let c2 = count.clone();

        let a = ensure_running("t1", &key, script.to_str(), &[], Duration::from_secs(30), &p1, move || {
            let c = c1.clone();
            async move { c.exists() }
        });
        let b = ensure_running("t2", &key, script.to_str(), &[], Duration::from_secs(30), &p2, move || {
            let c = c2.clone();
            async move { c.exists() }
        });

        let (ra, rb) = tokio::join!(a, b);
        ra.unwrap();
        rb.unwrap();

        let lines = std::fs::read_to_string(&count).unwrap().lines().count();
        assert_eq!(lines, 1, "排他により spawn は1回だけであること");

        terminate_process("t1", &p1);
        terminate_process("t2", &p2);
    }

    /// ロックを取れないまま timeout した場合は、警告のうえ従来どおりの経路に進むこと
    /// （待ち続けてハングしない）。
    #[tokio::test]
    async fn ensure_running_falls_through_when_lock_is_unavailable() {
        let key = unique_key("busy");
        let _held = crate::lock::try_acquire(&key).unwrap().unwrap();

        let process: Mutex<Option<EngineProcess>> = Mutex::new(None);
        let start = std::time::Instant::now();

        ensure_running("test", &key, None, &[], Duration::from_secs(1), &process, || async { true })
            .await
            .unwrap();

        assert!(start.elapsed() >= Duration::from_secs(1), "ロック待ちを経てからフォールスルーすること");
    }
```

- [ ] **Step 2: テストが失敗することを確認する**

Run: `cargo test -p s2v-engines --lib process::tests::concurrent_ensure_running_spawns_engine_only_once`
Expected: FAIL — `assertion \`left == right\` failed: 排他により spawn は1回だけであること
  left: 2, right: 1`（排他がまだ無いため2回 spawn される）

- [ ] **Step 3: 実装を書く**

`crates/s2v-engines/src/process.rs` の import に追加する。

```rust
use crate::lock;
```

`ensure_running` の本体の先頭（`let job = EngineJob::open_or_create(...)` の**前**）に挿入する。

```rust
    // 「起動確認 → spawn → 起動完了待ち」を跨いで排他する。ここを覆わないと、
    // P1 が spawn した直後（まだ /version が応答しない間）に P2 が来て二重に spawn してしまう。
    // ロックガードはこの関数を抜けるまで保持する。
    let startup_lock = lock::acquire(key, timeout).await;
    if startup_lock.is_none() {
        warn!("[{name}] 起動ロックを取得できませんでした。排他せずに続行します。");
    }
```

- [ ] **Step 4: テストが通ることを確認する**

Run: `cargo test -p s2v-engines`
Expected: 全 PASS

- [ ] **Step 5: コミット**

```bash
git add crates/s2v-engines/src/process.rs
git commit -m "fix(engines): エンジン起動処理をプロセス間排他し二重spawnを防ぐ"
```

---

## Task 5: ワークスペース全体の検証と記録

**Files:**
- Modify: `D:\ObsidianVault\ClaudeMemory\Projects\Script2Voice-Rust版\進捗.md`
- Modify: `D:\ObsidianVault\ClaudeMemory\Projects\Script2Voice-Rust版\失敗・教訓ログ.md`

**Interfaces:**
- Consumes: Task 1〜4 の全成果
- Produces: なし（検証と記録のみ）

- [ ] **Step 1: ワークスペース全体のテストを実行する**

Run: `cargo test --workspace --all-targets`
Expected: 全 PASS（既存の `s2v-video` / `s2v-gui` / `s2v-core` を含む）

- [ ] **Step 2: リリースビルドが通ることを確認する**

Run: `cargo build --release`
Expected: エラーなし。`dead_code` 警告が残っていないこと（Task 3 で出ていた `lock` の警告が
Task 4 で解消されているはず）。

- [ ] **Step 3: 実機で複数起動を検証する（手動）**

1. 既存の VOICEVOX / AivisSpeech を全て終了しておく（タスクマネージャで `run.exe` / `engine.exe` が
   残っていないことを確認）。
2. `target\release\script2voice.exe` を使い、**別々の台本ディレクトリで2プロセスを同時に起動**する。
   片方は短い台本（先に終わる側）、もう片方は長い台本にすると、今回のバグ条件を再現しやすい。
3. 短い方が完了した後も、長い方が最後まで完走することを確認する。
4. 両方の `run.log` に `error sending request for url` が**1件も出ていない**こと。

```bash
grep -c "error sending request" "<短い台本のディレクトリ>/run.log" "<長い台本のディレクトリ>/run.log"
```

Expected: 両方 `0`

5. 両プロセスが終了した後、タスクマネージャで engine.exe / run.exe が残っていないことを確認する。

- [ ] **Step 4: Beads を更新する**

```bash
bd close s2v-zm9
bd remember --key s2v-engine-multiprocess-sharing "エンジンのマルチプロセス共有を実装(s2v-zm9)。従来はプロセスごとの無名Job Object+明示TerminateJobObjectだったため、先に終了したプロセスが他プロセス使用中の共有エンジンを殺し、実行途中で『error sending request for url』が連続発生して音声が欠落していた。対策: (1)crates/s2v-engines/src/job.rs の EngineJob::new() を open_or_create(name) に変え、名前付きJob(Local\Script2Voice_Engine_<key>)を全プロセスが保持。key は process.rs の engine_resource_key(name,url)(URLのポートを含める)。(2)terminate_process から TerminateJobObject と Child::kill を廃止し、ハンドルを閉じるだけにした。最後の保持者が抜けた時点で KILL_ON_JOB_CLOSE により OS がエンジンツリーを終了する。EngineProcess.child は自分でspawnした時だけ Some。(3)起動レースの排他は named mutex ではなく crates/s2v-engines/src/lock.rs のロックファイル(OpenOptionsExt::share_mode(0)=FILE_SHARE_NONE)。named mutex はスレッドアフィニティがあり .await をまたぐ tokio では使えないため。プロセスが落ちればOSがハンドルを閉じ自動解放される。ハマりどころ: テストは同一プロセス内のスレッドで並列実行されるため、Job名・ロックキーはテストごとに一意にしないと相互干渉する(unique_job_name/unique_key ヘルパー)。コミット: <ハッシュを記入>。設計: docs/superpowers/specs/2026-09-04-engine-multiprocess-sharing-design.md 計画: docs/superpowers/plans/2026-09-04-engine-multiprocess-sharing.md"
```

- [ ] **Step 5: Obsidian に記録する**

`進捗.md` の先頭付近（`tags` ブロックの直後）に、今回の実装内容・設計判断・検証結果を追記する。

`失敗・教訓ログ.md` の先頭付近に、次の教訓を①症状 ②根本原因 ③教訓 ④関連Beads/コミット の形式で追記する。

- **症状**: Script2Voice を複数起動すると実行途中で `error sending request for url` が連続発生し、
  該当行の音声が欠落したまま処理が継続していた。
- **根本原因**: エンジン共有のヘルスチェック（A案）は実装済みだったのに、後始末（B案）が
  「プロセスごとの無名 Job Object ＋ 明示 `TerminateJobObject`」のままだった。共有を前提にした
  検知だけを入れ、生存期間の共有を入れなかったため、先に終了したプロセスが他プロセスの
  使用中のエンジンを殺していた。
- **教訓**: 資源を「共有して使い回す」設計に変えるときは、**取得側（検知・使い回し）と解放側
  （終了処理）を必ずセットで見直す**。取得側だけを共有対応にすると、解放側が単独所有のままで
  最も見つけにくい形（実行途中の断続的な失敗）で壊れる。
- **教訓2**: 引き継ぎ書・設計メモの「現状こうなっているはず」という推測は、着手前に必ず実コードと
  実ログで裏を取る。今回は引き継ぎ書が「毎回 spawn している／ポート競合が主因」と推測していたが、
  実際は A案・B案とも実装済みで、原因はまったく別（共有エンジンの道連れ終了）だった。
  `run.log` のタイムスタンプ突き合わせが決め手になった。

- [ ] **Step 6: コミット**

```bash
git add docs/superpowers/specs/2026-09-04-engine-multiprocess-sharing-design.md docs/superpowers/plans/2026-09-04-engine-multiprocess-sharing.md
git commit -m "docs: エンジンのマルチプロセス共有の設計書と実装計画を追加する"
```

---

## 完了条件

- `cargo test --workspace --all-targets` が全 PASS。
- `assigned_process_survives_until_last_handle_is_dropped`（共有中のエンジンが殺されない）と
  `concurrent_ensure_running_spawns_engine_only_once`（二重 spawn の防止）が PASS。
- 実機で2プロセス同時実行し、両方の `run.log` に `error sending request` が0件。
- 全プロセス終了後に engine.exe / run.exe が残らない。
- bead `s2v-zm9` がクローズされ、Obsidian の進捗ノート・教訓ノートが更新されている。
