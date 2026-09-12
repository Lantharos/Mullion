use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use crate::{
    PrepareProgress, PrepareStage, ServiceError, ServiceResult, SystemReleaseManifest,
    registry::replace_file,
};

use super::SERVICE_REPO;

pub(super) fn system_target() -> String {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    let arch = if cfg!(target_arch = "x86_64") {
        "x86_64"
    } else if cfg!(target_arch = "aarch64") {
        "aarch64"
    } else {
        "unknown"
    };
    format!("{os}-{arch}")
}

pub(super) fn system_asset_name() -> String {
    let extension = if cfg!(target_os = "windows") {
        "zip"
    } else {
        "tar.gz"
    };
    format!("sabine-system-{}.{}", system_target(), extension)
}

pub(super) fn sabine_host_relative_path() -> PathBuf {
    if cfg!(target_os = "windows") {
        PathBuf::from("sabine-host.exe")
    } else if cfg!(target_os = "macos") {
        PathBuf::from("sabine-host.app/Contents/MacOS/sabine-host")
    } else {
        PathBuf::from("sabine-host")
    }
}

pub(super) fn copy_directory(source: &Path, destination: &Path) -> ServiceResult<()> {
    if !source.is_dir() {
        return Err(ServiceError::Update(format!(
            "offline Sabine system bundle is missing {}",
            source.display()
        )));
    }
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_directory(&source_path, &destination_path)?;
        } else {
            fs::copy(source_path, destination_path)?;
        }
    }
    Ok(())
}

pub(super) fn service_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sabine-service.exe"
    } else {
        "sabine-service"
    }
}

pub(super) fn service_daemon_binary_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "sabine-service-daemon.exe"
    } else {
        "sabine-service-daemon"
    }
}

pub(super) fn which(name: &str) -> Result<PathBuf, ()> {
    let path = std::env::var_os("PATH").ok_or(())?;
    for directory in std::env::split_paths(&path) {
        let candidate = directory.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(())
}

pub(super) fn download_file(
    url: &str,
    destination: &Path,
    size: u64,
    on_progress: &mut impl FnMut(PrepareProgress),
) -> ServiceResult<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = destination.with_extension("download");
    sabine_runtime::download_file_with_progress(
        url,
        &temporary,
        Some(size),
        8 * 1024 * 1024 * 1024,
        &mut |update| {
            let portion = update.downloaded as f32 / size as f32;
            on_progress(PrepareProgress {
                stage: PrepareStage::Service,
                message: format!("Downloading Sabine service ({:.0}%)", portion * 100.0),
                fraction: Some(0.02 + portion * 0.06),
            });
        },
    )?;
    replace_file(&temporary, destination)?;
    Ok(())
}

pub(super) fn fetch_system_manifest(
    required_version: Option<&str>,
) -> ServiceResult<SystemReleaseManifest> {
    let url = std::env::var("SABINE_RELEASE_MANIFEST_URL")
        .unwrap_or_else(|_| system_manifest_url(required_version));
    if !url.starts_with("https://") {
        return Err(ServiceError::Update(
            "Sabine release manifest must use HTTPS".to_string(),
        ));
    }
    crate::http::fetch_manifest(&url)
}

fn system_manifest_url(required_version: Option<&str>) -> String {
    required_version.map_or_else(
        || {
            format!(
                "https://github.com/{SERVICE_REPO}/releases/latest/download/sabine-release.json"
            )
        },
        |version| {
            format!(
                "https://github.com/{SERVICE_REPO}/releases/download/v{version}/sabine-release.json"
            )
        },
    )
}

pub(super) fn verify_sha256(path: &Path, expected: &str) -> ServiceResult<()> {
    let mut input = fs::File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        digest.update(&buffer[..read]);
    }
    let actual = digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    if actual.eq_ignore_ascii_case(expected) {
        Ok(())
    } else {
        Err(ServiceError::Update(format!(
            "Sabine system bundle hash mismatch: expected {expected}, got {actual}"
        )))
    }
}

pub(super) fn extract_system_archive(archive: &Path, destination: &Path) -> ServiceResult<()> {
    crate::archive::extract(archive, destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn required_updates_keep_the_exact_release_tag() {
        assert!(system_manifest_url(None).contains("/releases/latest/"));
        assert!(system_manifest_url(Some("0.1.20")).contains("/releases/download/v0.1.20/"));
        assert!(system_manifest_url(Some("0.21")).contains("/releases/download/v0.21/"));
    }
}
