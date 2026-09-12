use std::{fs, io, path::Path};

use super::{BundleFormat, StagedBundle, copy_binary, copy_dir_recursive};

pub(super) fn validate_platform(format: BundleFormat, binary: &Path) -> Result<(), String> {
    let operating_system = match format {
        BundleFormat::Macos | BundleFormat::Dmg => "macos",
        BundleFormat::Windows | BundleFormat::Msi | BundleFormat::Exe => "windows",
        _ => "linux",
    };
    if operating_system != std::env::consts::OS {
        return Err("offline bundles must be built on their target operating system".into());
    }
    let architecture = match format {
        BundleFormat::Macos | BundleFormat::Dmg => "aarch64",
        BundleFormat::Windows | BundleFormat::Msi | BundleFormat::Exe => {
            windows_architecture(binary)?
        }
        _ => super::super::linux_package::architecture(binary)?.1,
    };
    if architecture != std::env::consts::ARCH {
        return Err(format!(
            "offline {architecture} bundles must be built on a {architecture} machine"
        ));
    }
    Ok(())
}

fn windows_architecture(binary: &Path) -> Result<&'static str, String> {
    use std::io::{Read, Seek, SeekFrom};
    let inspect = || -> io::Result<[u8; 6]> {
        let mut file = fs::File::open(binary)?;
        let mut dos = [0u8; 64];
        file.read_exact(&mut dos)?;
        if &dos[..2] != b"MZ" {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "missing DOS header",
            ));
        }
        let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap());
        file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut pe = [0u8; 6];
        file.read_exact(&mut pe)?;
        Ok(pe)
    };
    let pe = inspect().map_err(|error| format!("could not inspect Windows executable: {error}"))?;
    if &pe[..4] != b"PE\0\0" {
        return Err("Windows bundles require a PE executable".into());
    }
    match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => Ok("x86_64"),
        0xaa64 => Ok("aarch64"),
        _ => Err("Windows bundles support x86_64 and aarch64".into()),
    }
}

pub(super) fn stage_offline_runtime(
    format: BundleFormat,
    staged: &StagedBundle,
) -> Result<(), String> {
    let runtime = sabine_runtime::ensure_runtime(&sabine_runtime::RuntimeConfig::default())
        .map_err(|error| format!("could not prepare offline CEF runtime: {error}"))?;
    let host = sabine_host::ensure_host(runtime.location.path())?;
    let service = sabine_service::ensure_service_executable(|_| {})
        .map_err(|error| format!("could not prepare offline Sabine service: {error}"))?;
    let daemon = sabine_service::service_daemon_path(&service);
    let binary_dir = staged
        .binary
        .parent()
        .ok_or("offline application has no binary directory")?;
    let manifest_dir = match format {
        BundleFormat::Macos | BundleFormat::Dmg => staged.app_dir.join("Contents/Resources"),
        _ => binary_dir.join("resources"),
    };
    fs::create_dir_all(binary_dir).map_err(|error| error.to_string())?;
    for (source, name) in [
        (&service, service.file_name().unwrap_or_default()),
        (&daemon, daemon.file_name().unwrap_or_default()),
    ] {
        copy_binary(source, &binary_dir.join(name))?;
    }
    if cfg!(target_os = "macos") {
        let host_bundle = host
            .ancestors()
            .find(|path| path.extension().is_some_and(|extension| extension == "app"))
            .ok_or_else(|| "macOS Sabine host is not inside an app bundle".to_string())?;
        copy_dir_recursive(host_bundle, &binary_dir.join("sabine-host.app"))
            .map_err(|error| format!("could not stage macOS Sabine host: {error}"))?;
    } else {
        copy_binary(
            &host,
            &binary_dir.join(host.file_name().unwrap_or_default()),
        )?;
        if cfg!(windows) {
            copy_binary(
                &host.with_extension("dll"),
                &binary_dir.join("sabine-host.dll"),
            )?;
            copy_binary(
                &host.with_file_name("chrome_elf.dll"),
                &binary_dir.join("chrome_elf.dll"),
            )?;
        }
    }
    let runtime_name = runtime
        .location
        .path()
        .file_name()
        .ok_or_else(|| "offline runtime has no directory name".to_string())?;
    copy_runtime_payload(
        runtime.location.path(),
        &manifest_dir.join("runtimes/cef").join(runtime_name),
    )
    .map_err(|error| format!("could not stage offline CEF runtime: {error}"))
}

fn copy_runtime_payload(source: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    let source = source.canonicalize()?;
    for name in [
        "Release",
        "Resources",
        "Chromium Embedded Framework.framework",
        ".sabine-version",
        "LICENSE.txt",
        "README.txt",
    ] {
        let path = source.join(name);
        if path.is_dir() {
            copy_runtime_recursive(&path, &destination.join(name), &source)?;
        } else if path.is_file() {
            fs::copy(&path, destination.join(name))?;
        }
    }
    Ok(())
}

fn copy_runtime_recursive(source: &Path, destination: &Path, root: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let name_text = name.to_string_lossy();
        if matches!(
            name_text.as_ref(),
            ".leases" | ".sabine-host-build" | ".sabine-hosts"
        ) || name_text.ends_with(".installing")
        {
            continue;
        }
        let source_path = entry.path();
        let destination_path = destination.join(name);
        if entry.file_type()?.is_symlink() {
            let target = fs::read_link(&source_path)?;
            if target.is_absolute() || !source_path.canonicalize()?.starts_with(root) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "runtime symlink escapes the bundle",
                ));
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, destination_path)?;
            #[cfg(windows)]
            if source_path.is_dir() {
                std::os::windows::fs::symlink_dir(target, destination_path)?;
            } else {
                std::os::windows::fs::symlink_file(target, destination_path)?;
            }
        } else if source_path.is_dir() {
            copy_runtime_recursive(&source_path, &destination_path, root)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}
