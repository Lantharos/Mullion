use std::{
    collections::hash_map::DefaultHasher,
    fs,
    hash::{Hash, Hasher},
    os::unix::fs::{PermissionsExt, symlink},
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

pub(super) fn prepare(host: &Path, runtime: &Path) -> Result<PathBuf, String> {
    let bundle = host
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .ok_or("macOS host must be inside an application bundle")?;
    let relative_host = host
        .strip_prefix(bundle)
        .map_err(|error| error.to_string())?;
    let framework = [runtime.join("Release"), runtime.to_path_buf()]
        .into_iter()
        .map(|root| root.join("Chromium Embedded Framework.framework"))
        .find(|path| path.is_dir())
        .ok_or("Chromium framework is missing from the selected runtime")?;
    let metadata = host.metadata().map_err(|error| error.to_string())?;
    let mut fingerprint = DefaultHasher::new();
    host.hash(&mut fingerprint);
    metadata.len().hash(&mut fingerprint);
    metadata
        .modified()
        .map_err(|error| error.to_string())?
        .hash(&mut fingerprint);
    let framework_metadata = framework
        .join("Chromium Embedded Framework")
        .metadata()
        .map_err(|error| error.to_string())?;
    framework_metadata.len().hash(&mut fingerprint);
    framework_metadata
        .modified()
        .map_err(|error| error.to_string())?
        .hash(&mut fingerprint);
    let cache =
        sabine_runtime::runtime_execution_path(runtime).map_err(|error| error.to_string())?;
    let directory = cache.join(format!(
        "{}-{:016x}",
        crate::host_source_fingerprint(),
        fingerprint.finish()
    ));
    let executable = directory.join("sabine-host.app").join(relative_host);
    if directory.join("ready").is_file() && executable.is_file() {
        return Ok(executable);
    }
    let _lock = sabine_runtime::FileLock::acquire(
        &cache.join(".assembly.lock"),
        Duration::from_secs(600),
        |_| {},
    )
    .map_err(|error| error.to_string())?;
    if directory.join("ready").is_file() && executable.is_file() {
        return Ok(executable);
    }
    let staging = directory.with_extension("installing");
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(|error| error.to_string())?;
    }
    let staged_bundle = staging.join("sabine-host.app");
    let result = (|| {
        copy_tree(bundle, &staged_bundle, false).map_err(|error| error.to_string())?;
        copy_tree(
            &framework,
            &staged_bundle.join("Contents/Frameworks/Chromium Embedded Framework.framework"),
            true,
        )
        .map_err(|error| error.to_string())?;
        crate::run_checked(
            Command::new("/usr/bin/codesign")
                .args(["--force", "--sign", "-", "--timestamp=none"])
                .arg(&staged_bundle),
        )?;
        crate::run_checked(
            Command::new("/usr/bin/codesign")
                .args(["--verify", "--deep", "--strict"])
                .arg(&staged_bundle),
        )?;
        fs::write(staging.join("ready"), []).map_err(|error| error.to_string())?;
        sabine_runtime::install_directory(&staging, &directory)
            .map_err(|error| error.to_string())?;
        Ok(executable)
    })();
    if result.is_err() {
        let _ = fs::remove_dir_all(staging);
    }
    result
}

fn copy_tree(source: &Path, destination: &Path, shared: bool) -> std::io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        let kind = entry.file_type()?;
        if kind.is_symlink() {
            symlink(fs::read_link(&from)?, to)?;
        } else if kind.is_dir() {
            copy_tree(&from, &to, shared)?;
        } else if shared {
            if let Err(error) = fs::hard_link(&from, &to) {
                if !matches!(
                    error.kind(),
                    std::io::ErrorKind::CrossesDevices
                        | std::io::ErrorKind::PermissionDenied
                        | std::io::ErrorKind::Unsupported
                ) {
                    return Err(error);
                }
                fs::copy(from, to)?;
            }
        } else {
            fs::copy(&from, &to)?;
            let permissions = fs::metadata(&to)?.permissions().mode() | 0o200;
            fs::set_permissions(&to, fs::Permissions::from_mode(permissions))?;
        }
    }
    Ok(())
}
