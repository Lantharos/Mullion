use std::{fs, path::PathBuf, time::Duration};

use sabine_runtime::FileLock;
use serde::{Deserialize, Serialize};

use super::{PID_FILE, process_alive, process_executable, same_executable};
use crate::{ServiceResult, service_data_dir};

pub(super) const DAEMON_STATE_FILE: &str = "daemon-state.json";

pub(super) struct DaemonPid {
    path: PathBuf,
    state_path: PathBuf,
    pub pid: u32,
    _lock: FileLock,
}

impl Drop for DaemonPid {
    fn drop(&mut self) {
        let owns_file = fs::read_to_string(&self.path)
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
            == Some(self.pid);
        if owns_file {
            let _ = fs::remove_file(&self.path);
        }
        if daemon_state().is_some_and(|state| state.pid == self.pid) {
            let _ = fs::remove_file(&self.state_path);
        }
    }
}

pub(super) fn claim_daemon_pid() -> ServiceResult<Option<DaemonPid>> {
    let directory = service_data_dir();
    let lock = match FileLock::acquire(&directory.join("daemon.lock"), Duration::ZERO, |_| {}) {
        Ok(lock) => lock,
        Err(error) if error.kind() == std::io::ErrorKind::TimedOut => return Ok(None),
        Err(error) => return Err(error.into()),
    };
    let path = directory.join(PID_FILE);
    if let Some(pid) = fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<u32>().ok())
        .filter(|pid| *pid <= i32::MAX as u32 && process_alive(*pid as i32))
        && let Some(actual) = process_executable(pid)
    {
        let current = std::env::current_exe()?;
        let registered = daemon_state()
            .filter(|state| state.pid == pid)
            .map(|state| {
                crate::service_daemon_path(&crate::install::service_path_for_version(
                    &state.version,
                ))
            });
        if same_executable(&actual, &current)
            || registered.is_some_and(|expected| same_executable(&actual, &expected))
        {
            return Ok(None);
        }
    }
    let owner = DaemonPid {
        path,
        state_path: directory.join(DAEMON_STATE_FILE),
        pid: std::process::id(),
        _lock: lock,
    };
    fs::write(&owner.path, owner.pid.to_string())?;
    fs::write(
        &owner.state_path,
        serde_json::to_vec(&DaemonState {
            pid: owner.pid,
            version: crate::SABINE_VERSION.to_string(),
        })
        .expect("daemon state is serializable"),
    )?;
    Ok(Some(owner))
}

#[derive(Deserialize, Serialize)]
pub(super) struct DaemonState {
    pub pid: u32,
    pub version: String,
}

pub(super) fn daemon_state() -> Option<DaemonState> {
    serde_json::from_slice(&fs::read(service_data_dir().join(DAEMON_STATE_FILE)).ok()?).ok()
}
