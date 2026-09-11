use std::{fs, io, path::Path};

/// Publishes a validated directory and retains the previous contents until the
/// replacement is in place. The caller must hold its installation lock.
pub fn install_directory(staging: &Path, destination: &Path) -> io::Result<()> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("installation has no parent"))?;
    let name = destination
        .file_name()
        .ok_or_else(|| invalid("installation has no name"))?;
    if staging == destination || !staging.is_dir() {
        return Err(invalid("installation staging must be a separate directory"));
    }
    recover_directory_installs(parent)?;
    let rollback_root = parent.join(".rollback");
    fs::create_dir_all(&rollback_root)?;
    let previous = rollback_root.join(name);
    if destination.exists() {
        fs::rename(destination, &previous)?;
    }
    let publish = (|| {
        sync_directory(&rollback_root)?;
        sync_directory(parent)?;
        fs::rename(staging, destination)?;
        sync_directory(parent)
    })();
    if let Err(error) = publish {
        if !destination.exists() && previous.exists() {
            fs::rename(&previous, destination).map_err(|restore| {
                io::Error::other(format!(
                    "installation failed: {error}; restoring {} failed: {restore}",
                    destination.display()
                ))
            })?;
            sync_directory(parent)?;
        }
        return Err(error);
    }
    if previous.exists()
        && let Err(error) = fs::remove_dir_all(&previous)
    {
        crate::report_error(
            "installation",
            format!(
                "installed {}, but could not remove its previous directory: {error}",
                destination.display()
            ),
        );
    }
    Ok(())
}

/// Completes cleanup or restores a directory interrupted before publication.
/// The caller must hold the same lock used for directory installation.
pub fn recover_directory_installs(parent: &Path) -> io::Result<()> {
    let rollback = parent.join(".rollback");
    if !rollback.is_dir() {
        return Ok(());
    }
    for entry in fs::read_dir(&rollback)? {
        let entry = entry?;
        let destination = parent.join(entry.file_name());
        if destination.exists() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::rename(entry.path(), &destination)?;
        }
        sync_directory(parent)?;
        sync_directory(&rollback)?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> io::Result<()> {
    #[cfg(unix)]
    fs::File::open(path)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
