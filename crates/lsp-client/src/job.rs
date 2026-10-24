//! Windows Job Object that kills the spawned Godot when the IDE exits,
//! including on a crash, since the OS closes our handle either way.

use std::os::windows::io::AsRawHandle;
use std::process::Child;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};

/// Stored as an integer so the owning client stays `Send + Sync`.
pub struct Job(isize);

impl Job {
    pub fn assign(child: &Child) -> Option<Job> {
        // SAFETY: plain Win32 calls on handles we own; `info` outlives the call.
        unsafe {
            let handle: HANDLE = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return None;
            }
            let job = Job(handle as isize);
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let ok = SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                std::mem::size_of_val(&info) as u32,
            );
            if ok == 0 || AssignProcessToJobObject(handle, child.as_raw_handle() as HANDLE) == 0 {
                return None;
            }
            Some(job)
        }
    }
}

impl Drop for Job {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateJobObjectW and is closed once.
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}
