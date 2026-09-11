use std::{
    io,
    os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle},
    process::Command,
    sync::atomic::{AtomicBool, Ordering},
};

use windows::Win32::{
    Foundation::{HANDLE, HWND, LPARAM, WPARAM},
    System::{
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
            JobObjectExtendedLimitInformation, SetInformationJobObject, TerminateJobObject,
        },
        Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE},
    },
    UI::WindowsAndMessaging::{EnumWindows, GetWindowThreadProcessId, PostMessageW, WM_CLOSE},
};

pub(super) struct ProcessGroup {
    id: u32,
    job: OwnedHandle,
    active: AtomicBool,
}

impl ProcessGroup {
    pub(super) fn register(id: u32) -> io::Result<Self> {
        unsafe {
            let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, id)?;
            let process = OwnedHandle::from_raw_handle(process.0);
            let job = CreateJobObjectW(None, None)?;
            let job = OwnedHandle::from_raw_handle(job.0);
            let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            limits.BasicLimitInformation.LimitFlags =
                JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE | JOB_OBJECT_LIMIT_SILENT_BREAKAWAY_OK;
            SetInformationJobObject(
                HANDLE(job.as_raw_handle()),
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )?;
            AssignProcessToJobObject(HANDLE(job.as_raw_handle()), HANDLE(process.as_raw_handle()))?;
            Ok(Self {
                id,
                job,
                active: AtomicBool::new(true),
            })
        }
    }

    pub(super) fn terminate(&self) {
        if self.active.load(Ordering::Acquire) {
            unsafe {
                let _ = EnumWindows(Some(close_window), LPARAM(self.id as isize));
            }
        }
    }

    pub(super) fn kill(&self) {
        if self.active.load(Ordering::Acquire) {
            unsafe {
                let _ = TerminateJobObject(HANDLE(self.job.as_raw_handle()), 1);
            }
        }
    }

    pub(super) fn unregister(&self) {
        self.active.store(false, Ordering::Release);
    }
}

unsafe extern "system" fn close_window(window: HWND, process_id: LPARAM) -> windows::core::BOOL {
    let mut owner = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut owner));
        if owner == process_id.0 as u32 {
            let _ = PostMessageW(Some(window), WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
    true.into()
}

pub(super) fn prepare_child_command(command: &mut Command, _die_with_parent: bool) {
    sabine_runtime::configure_background_command(command);
}
