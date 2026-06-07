use std::future::Future;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::Duration;

use tracing::info;

/// 起動待機のポーリング間隔と最大試行回数（Python 版の `for i in range(30): await sleep(1)` に合わせる）。
const POLL_INTERVAL: Duration = Duration::from_secs(1);
const POLL_RETRIES: usize = 30;

/// 接続確認を行い、失敗時は `exe_path` が設定されていればプロセスを起動して
/// 起動完了まで待機する。既に起動済みなら何もしない。
pub(crate) async fn ensure_running<F, Fut>(
    name: &str,
    exe_path: Option<&str>,
    process: &Mutex<Option<Child>>,
    is_alive: F,
) -> anyhow::Result<()>
where
    F: Fn() -> Fut,
    Fut: Future<Output = bool>,
{
    if is_alive().await {
        info!("[{name}] 既に起動しています。");
        return Ok(());
    }

    let path = exe_path.ok_or_else(|| {
        anyhow::anyhow!("{name}: サーバーに接続できず、exe_path も未設定のため起動できません")
    })?;
    if !std::path::Path::new(path).exists() {
        anyhow::bail!("{name}: 実行ファイルが見つかりません: {path}");
    }

    info!("[{name}] 起動を確認できません。プロセスを起動します: {path}");
    let child = Command::new(path)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| anyhow::anyhow!("{name}: プロセスの起動に失敗しました: {e}"))?;
    *process.lock().unwrap() = Some(child);

    for _ in 0..POLL_RETRIES {
        tokio::time::sleep(POLL_INTERVAL).await;
        if is_alive().await {
            info!("[{name}] エンジンの起動を確認しました。");
            return Ok(());
        }
    }
    anyhow::bail!("{name}: 起動待機がタイムアウトしました")
}

/// activate() でプロセスを起動していた場合、それを終了する。起動していなければ何もしない。
pub(crate) fn terminate_process(name: &str, process: &Mutex<Option<Child>>) {
    let mut guard = process.lock().unwrap();
    if let Some(mut child) = guard.take() {
        info!("[{name}] エンジンプロセスを停止します。");
        if child.kill().is_ok() {
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::Arc;

    /// cmd.exe の `%~dp0` でバッチファイル自身のディレクトリを解決させることで、
    /// 日本語ユーザー名を含む一時ディレクトリでもパスのエンコード崩れを避ける。
    fn write_marker_script(dir: &std::path::Path) -> std::path::PathBuf {
        let script = dir.join("fake_engine.cmd");
        std::fs::write(&script, "@echo off\r\necho ready > \"%~dp0marker.txt\"\r\n").unwrap();
        script
    }

    #[tokio::test]
    async fn ensure_running_does_not_spawn_when_already_alive() {
        let process: Mutex<Option<Child>> = Mutex::new(None);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls2 = Arc::clone(&calls);

        ensure_running("test", None, &process, move || {
            calls2.fetch_add(1, Ordering::SeqCst);
            async { true }
        })
        .await
        .unwrap();

        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(process.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn ensure_running_errors_when_not_alive_and_no_exe_path() {
        let process: Mutex<Option<Child>> = Mutex::new(None);

        let result = ensure_running("test", None, &process, || async { false }).await;

        assert!(result.is_err());
    }

    #[tokio::test]
    async fn ensure_running_errors_when_exe_path_does_not_exist() {
        let process: Mutex<Option<Child>> = Mutex::new(None);

        let result = ensure_running("test", Some("C:/no/such/engine.exe"), &process, || async { false }).await;

        assert!(result.is_err());
        assert!(process.lock().unwrap().is_none());
    }

    #[tokio::test]
    async fn ensure_running_spawns_process_and_waits_until_alive() {
        let dir = tempfile::tempdir().unwrap();
        let script = write_marker_script(dir.path());
        let marker = dir.path().join("marker.txt");
        assert!(!marker.exists());

        let process: Mutex<Option<Child>> = Mutex::new(None);
        let marker_for_check = marker.clone();

        ensure_running("test", script.to_str(), &process, move || {
            let marker = marker_for_check.clone();
            async move { marker.exists() }
        })
        .await
        .unwrap();

        assert!(marker.exists(), "起動したプロセスがマーカーファイルを作成していること");
        assert!(process.lock().unwrap().is_some(), "起動したプロセスが保持されていること");

        terminate_process("test", &process);
    }

    #[tokio::test]
    async fn terminate_process_kills_running_process_and_clears_handle() {
        let mut child = std::process::Command::new("cmd")
            .args(["/c", "ping", "-n", "60", "127.0.0.1"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        assert!(child.try_wait().unwrap().is_none(), "プロセスが起動していること");

        let process: Mutex<Option<Child>> = Mutex::new(Some(child));
        terminate_process("test", &process);

        assert!(process.lock().unwrap().is_none(), "ハンドルが解放されていること");
    }

    #[test]
    fn terminate_process_is_noop_when_nothing_was_spawned() {
        let process: Mutex<Option<Child>> = Mutex::new(None);
        // パニックしないことを確認する
        terminate_process("test", &process);
        let _ = AtomicBool::new(false);
    }
}
