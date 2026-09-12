pub(crate) mod transfer;

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

use serde::Deserialize;
use sha1::{Digest, Sha1};

use crate::error::RuntimeError;
use crate::paths::runtime_version_path;
use crate::types::{RuntimeConfig, RuntimeInstallPlan, RuntimeInstallProgress, RuntimeInstallStep};
use crate::version::{cef_platform_key, channel_preference, major_version, version_sort_key};

pub(crate) fn download_file(
    url: &str,
    destination: &Path,
    size: u64,
    progress: &mut impl FnMut(RuntimeInstallProgress),
) -> Result<(), RuntimeError> {
    transfer::download_file(url, destination, Some(size), size, &mut |update| {
        let portion = update.downloaded as f32 / size as f32;
        let message = if update.attempt > 1 {
            format!("Resuming runtime download (attempt {})", update.attempt)
        } else {
            format!("Downloading runtime ({:.0}%)", portion * 100.0)
        };
        progress(RuntimeInstallProgress::new(
            RuntimeInstallStep::Downloading,
            Some(0.05 + portion * 0.65),
            message,
        ));
    })
}

pub const DEFAULT_CEF_INDEX_URL: &str = "https://cef-builds.spotifycdn.com/index.json";

pub(crate) fn fetch_cef_index(index_url: &str) -> Result<CefIndex, RuntimeError> {
    let mut response = ureq::get(index_url)
        .config()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .timeout_connect(Some(std::time::Duration::from_secs(15)))
        .build()
        .call()
        .map_err(|error| RuntimeError::InstallationFailed(error.to_string()))?;
    let output = response
        .body_mut()
        .read_to_vec()
        .map_err(|error| RuntimeError::InstallationFailed(error.to_string()))?;
    serde_json::from_slice(&output)
        .map_err(|error| RuntimeError::InstallationFailed(error.to_string()))
}

pub(crate) fn archive_url(index_url: &str, archive_name: &str) -> String {
    if archive_name.starts_with("https://") || archive_name.starts_with("http://") {
        return archive_name.to_string();
    }

    let base = index_url
        .rsplit_once('/')
        .map(|(base, _)| base)
        .unwrap_or("");
    if base.is_empty() {
        archive_name.to_string()
    } else {
        format!("{base}/{archive_name}")
    }
}

pub(crate) fn verify_sha1_with_progress(
    path: &Path,
    expected: &str,
    progress: &mut impl FnMut(RuntimeInstallProgress),
) -> Result<(), RuntimeError> {
    let metadata = std::fs::metadata(path)?;
    let total = metadata.len().max(1);
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = [0_u8; 64 * 1024];
    let mut read_total = 0_u64;
    let mut last_report = std::time::Instant::now()
        .checked_sub(std::time::Duration::from_secs(1))
        .unwrap_or_else(std::time::Instant::now);
    loop {
        let read = std::io::Read::read(&mut file, &mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        read_total = read_total.saturating_add(read as u64);
        if last_report.elapsed() >= std::time::Duration::from_millis(100) {
            let portion = (read_total as f32 / total as f32).clamp(0.0, 1.0);
            progress(RuntimeInstallProgress::new(
                RuntimeInstallStep::Verifying,
                Some(0.72 + portion * 0.05),
                "Verifying runtime",
            ));
            last_report = std::time::Instant::now();
        }
    }
    let actual = hasher
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    actual
        .eq_ignore_ascii_case(expected)
        .then_some(())
        .ok_or_else(|| RuntimeError::IntegrityFailed {
            path: path.to_path_buf(),
        })?;
    progress(RuntimeInstallProgress::new(
        RuntimeInstallStep::Verifying,
        Some(0.78),
        "Verifying runtime",
    ));
    Ok(())
}

pub(crate) fn extract_archive(archive: &Path, destination: &Path) -> Result<(), RuntimeError> {
    let input = std::fs::File::open(archive)?;
    crate::archive::extract_tar(bzip2::read::BzDecoder::new(input), destination)?;
    Ok(())
}

pub(crate) fn first_extracted_runtime_dir(work_dir: &Path) -> Option<PathBuf> {
    std::fs::read_dir(work_dir)
        .ok()?
        .flatten()
        .find_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?;
            (path.is_dir() && name.starts_with("cef_binary_")).then_some(path)
        })
}

pub fn latest_install_plan(config: &RuntimeConfig) -> Result<RuntimeInstallPlan, RuntimeError> {
    let platform = cef_platform_key().ok_or_else(|| {
        RuntimeError::InstallationFailed("unsupported OS or CPU architecture for CEF".to_string())
    })?;
    let index_url = config.index_url.as_deref().unwrap_or(DEFAULT_CEF_INDEX_URL);
    let index = fetch_cef_index(index_url)?;
    let platform_index = index.platforms.get(platform).ok_or_else(|| {
        RuntimeError::InstallationFailed(format!("CEF index does not contain platform {platform}"))
    })?;
    let min_major = crate::MIN_CEF_MAJOR
        .split('.')
        .next()
        .and_then(|major| major.parse::<u32>().ok())
        .unwrap_or(0);

    let mut candidates = platform_index
        .versions
        .iter()
        .filter(|version| major_version(&version.cef_version) >= min_major)
        .filter_map(|version| {
            version
                .files
                .iter()
                .find(|file| file.kind == "minimal")
                .map(|file| (version, file))
        })
        .collect::<Vec<_>>();

    candidates.sort_by_key(|(version, _)| {
        (
            channel_preference(version.channel.as_deref()),
            std::cmp::Reverse(version_sort_key(&version.cef_version)),
        )
    });

    let Some((version, file)) = candidates.into_iter().next() else {
        return Err(RuntimeError::NotFound(format!(
            "no Minimal CEF build found for {platform} at Chromium {} or newer",
            crate::MIN_CEF_MAJOR,
        )));
    };

    if !safe_archive_name(&file.name)
        || !version
            .cef_version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '+' | '-' | '_'))
        || file.size == 0
        || file.size > 2 * 1024 * 1024 * 1024
        || file.sha1.len() != 40
        || !file.sha1.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RuntimeError::InstallationFailed(
            "CEF index contains invalid archive metadata".to_string(),
        ));
    }
    let install_dir = runtime_version_path(&version.cef_version);
    Ok(RuntimeInstallPlan {
        version: version.cef_version.clone(),
        platform: platform.to_string(),
        archive_name: file.name.clone(),
        url: archive_url(index_url, &file.name),
        sha1: file.sha1.clone(),
        archive_size: file.size,
        install_dir,
    })
}

fn safe_archive_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '+' | '-' | '_'))
}

#[derive(Deserialize)]
pub(crate) struct CefIndex {
    #[serde(flatten)]
    platforms: BTreeMap<String, CefPlatformIndex>,
}

#[derive(Deserialize)]
struct CefPlatformIndex {
    versions: Vec<CefVersion>,
}

#[derive(Deserialize)]
struct CefVersion {
    cef_version: String,
    #[serde(default)]
    channel: Option<String>,
    files: Vec<CefFile>,
}

#[derive(Deserialize)]
struct CefFile {
    size: u64,
    name: String,
    sha1: String,
    #[serde(rename = "type")]
    kind: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn archive_urls_follow_index_location() {
        assert_eq!(
            archive_url("https://example.com/cef/index.json", "cef.tar.bz2"),
            "https://example.com/cef/cef.tar.bz2"
        );
        assert_eq!(
            archive_url(
                "https://example.com/cef/index.json",
                "https://cdn.example/cef.tar.bz2"
            ),
            "https://cdn.example/cef.tar.bz2"
        );
    }

    #[test]
    fn runtime_download_uses_the_in_process_http_client() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let body = b"sabine-runtime";
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = std::io::Read::read(&mut stream, &mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            std::io::Write::write_all(&mut stream, body).unwrap();
        });
        let nonce = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let destination = std::env::temp_dir().join(format!(
            "sabine-runtime-download-{}-{nonce}",
            std::process::id()
        ));
        let mut progress = Vec::new();
        download_file(
            &format!("http://{address}/runtime"),
            &destination,
            body.len() as u64,
            &mut |update| progress.push(update),
        )
        .unwrap();
        server.join().unwrap();
        assert_eq!(std::fs::read(&destination).unwrap(), body);
        assert!(
            progress
                .iter()
                .any(|update| update.step == RuntimeInstallStep::Downloading)
        );
        std::fs::remove_file(destination).unwrap();
    }
}
