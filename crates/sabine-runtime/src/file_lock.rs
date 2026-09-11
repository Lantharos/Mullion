use std::{
    fs::{File, OpenOptions, TryLockError},
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

/// An installation lock released by the operating system when its owner exits.
pub struct FileLock {
    _file: File,
}

impl FileLock {
    pub fn acquire(
        path: &Path,
        timeout: Duration,
        mut waiting: impl FnMut(Duration),
    ) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::Error(error)) => return Err(error),
                Err(TryLockError::WouldBlock) => {
                    let elapsed = started.elapsed();
                    if elapsed >= timeout {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            format!("timed out waiting for {}", path.display()),
                        ));
                    }
                    waiting(elapsed);
                    thread::sleep(Duration::from_millis(25).min(timeout - elapsed));
                }
            }
        }
    }
}
