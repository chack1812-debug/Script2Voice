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

    /// 指定したプロセスをこの Job に割り当てる。
    /// 以後そのプロセスが起動する子プロセス（孫プロセス）も同じ Job に属する。
    pub(crate) fn assign(&self, child: &Child) -> io::Result<()> {
        let process_handle = child.as_raw_handle() as HANDLE;
        let ok = unsafe { AssignProcessToJobObject(self.handle, process_handle) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    /// Job 配下の全プロセス（ツリー全体）を即座に終了する。
    pub(crate) fn terminate(&self) -> io::Result<()> {
        let ok = unsafe { TerminateJobObject(self.handle, 1) };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}

impl Drop for EngineJob {
    fn drop(&mut self) {
        // ハンドルを閉じる。terminate() を呼ばずにここに到達した場合
        // (例: 本体プロセスのクラッシュで最後のハンドルとして閉じられる場合)でも、
        // KILL_ON_JOB_CLOSE によりOSがJob配下のプロセスを自動的に終了する。
        unsafe { CloseHandle(self.handle) };
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
            // ここで terminate() を呼ばずに job をドロップする(クラッシュ相当の状況を模す)
        }

        std::thread::sleep(Duration::from_millis(300));
        assert!(
            child.try_wait().unwrap().is_some(),
            "Jobハンドルのクローズで自動終了していること(KILL_ON_JOB_CLOSE)"
        );
    }
}
