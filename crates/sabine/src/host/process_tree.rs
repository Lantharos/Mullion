use std::{
    process::{Child, Command, ExitStatus},
    time::Duration,
};

pub(crate) struct ManagedChild {
    id: u32,
    exit: crossbeam_channel::Receiver<std::io::Result<ExitStatus>>,
    group: ProcessGroup,
}

impl ManagedChild {
    pub(crate) fn new(
        mut child: Child,
        exited: crossbeam_channel::Sender<u32>,
    ) -> std::io::Result<Self> {
        let id = child.id();
        let group = match ProcessGroup::register(id) {
            Ok(group) => group,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        let (exit_sender, exit) = crossbeam_channel::bounded(1);
        std::thread::spawn(move || {
            let mut child = child;
            let status = child.wait();
            let _ = exit_sender.send(status);
            let _ = exited.send(id);
        });
        Ok(Self { id, exit, group })
    }

    pub(crate) fn id(&self) -> u32 {
        self.id
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        match self.exit.try_recv() {
            Ok(status) => self.finish(status).map(Some),
            Err(crossbeam_channel::TryRecvError::Empty) => Ok(None),
            Err(crossbeam_channel::TryRecvError::Disconnected) => Err(std::io::Error::other(
                "OSR host waiter exited without a status",
            )),
        }
    }

    pub(crate) fn terminate(&mut self) -> Option<ExitStatus> {
        self.group.terminate();
        match self.exit.recv_timeout(Duration::from_millis(250)) {
            Ok(status) => return self.finish(status).ok(),
            Err(crossbeam_channel::RecvTimeoutError::Disconnected) => return None,
            Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
        }
        self.group.kill();
        let status = self.exit.recv_timeout(Duration::from_secs(5)).ok()?.ok();
        self.group.unregister();
        status
    }

    fn finish(&self, status: std::io::Result<ExitStatus>) -> std::io::Result<ExitStatus> {
        self.group.unregister();
        status
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.group.unregister();
    }
}

pub(crate) fn prepare_child_command(command: &mut Command) {
    platform::prepare_child_command(command, true);
}

/// Prepare a child that may outlive this process when siblings still need it
/// (shared CEF process-singleton). Still gets a fresh process group.
pub(crate) fn prepare_detachable_child_command(command: &mut Command) {
    platform::prepare_child_command(command, false);
}

#[cfg(unix)]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(windows)]
#[path = "process_tree/windows.rs"]
mod platform;
#[cfg(windows)]
use platform::ProcessGroup;

#[cfg(unix)]
struct ProcessGroup {
    id: u32,
    active: AtomicBool,
}

#[cfg(unix)]
impl ProcessGroup {
    fn register(id: u32) -> std::io::Result<Self> {
        platform::register_process_group(id);
        Ok(Self {
            id,
            active: AtomicBool::new(true),
        })
    }

    fn terminate(&self) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        platform::terminate_process_group(self.id);
    }

    fn kill(&self) {
        if !self.active.load(Ordering::SeqCst) {
            return;
        }
        platform::kill_process_group(self.id);
    }

    fn unregister(&self) {
        if !self.active.swap(false, Ordering::SeqCst) {
            return;
        }
        platform::unregister_process_group(self.id);
    }
}

#[cfg(unix)]
mod platform {
    use std::{
        process::Command,
        sync::atomic::{AtomicBool, AtomicI32, Ordering},
    };

    const SIGINT: i32 = 2;
    const SIGKILL: i32 = 9;
    const SIGTERM: i32 = 15;
    const GROUP_CAPACITY: usize = 128;

    static INSTALLED: AtomicBool = AtomicBool::new(false);
    static PROCESS_GROUPS: [AtomicI32; GROUP_CAPACITY] =
        [const { AtomicI32::new(0) }; GROUP_CAPACITY];

    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
        fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
        fn _exit(status: i32) -> !;
    }

    pub(super) fn prepare_child_command(command: &mut Command, die_with_parent: bool) {
        use std::os::unix::process::CommandExt;

        install_signal_cleanup();
        command.process_group(0);
        #[cfg(target_os = "linux")]
        {
            use std::io;
            if !die_with_parent {
                return;
            }
            // DIE if the parent process exits unexpectedly so OSR hosts do not
            // outlive a crashed Sabine app. CEF itself is detachable so closing
            // one window does not SIGTERM the shared browser process.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) != 0 {
                        return Err(io::Error::last_os_error());
                    }
                    if libc::getppid() == 1 {
                        libc::raise(libc::SIGTERM);
                    }
                    Ok(())
                });
            }
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = die_with_parent;
        }
    }

    pub(super) fn install_signal_cleanup() {
        if INSTALLED.swap(true, Ordering::SeqCst) {
            return;
        }
        unsafe {
            signal(SIGINT, handle_signal);
            signal(SIGTERM, handle_signal);
        }
    }

    pub(super) fn register_process_group(id: u32) {
        let Ok(id) = i32::try_from(id) else {
            return;
        };
        if id <= 0 {
            return;
        }
        install_signal_cleanup();
        for slot in &PROCESS_GROUPS {
            if slot
                .compare_exchange(0, id, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
            {
                return;
            }
        }
    }

    pub(super) fn unregister_process_group(id: u32) {
        let Ok(id) = i32::try_from(id) else {
            return;
        };
        for slot in &PROCESS_GROUPS {
            let _ = slot.compare_exchange(id, 0, Ordering::SeqCst, Ordering::SeqCst);
        }
    }

    pub(super) fn terminate_process_group(id: u32) {
        send_process_group(id, SIGTERM);
    }

    pub(super) fn kill_process_group(id: u32) {
        send_process_group(id, SIGKILL);
    }

    extern "C" fn handle_signal(signal: i32) {
        for slot in &PROCESS_GROUPS {
            let id = slot.load(Ordering::SeqCst);
            if id > 0 {
                unsafe {
                    kill(-id, SIGTERM);
                    kill(-id, SIGKILL);
                }
            }
        }
        unsafe {
            _exit(128 + signal);
        }
    }

    fn send_process_group(id: u32, signal: i32) {
        let Ok(id) = i32::try_from(id) else {
            return;
        };
        if id <= 0 {
            return;
        }
        unsafe {
            kill(-id, signal);
        }
    }
}
