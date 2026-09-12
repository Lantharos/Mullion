use std::{fs, io, path::Path};

use sabine_platform::NativeMessagingHost;

pub(super) struct Manifests {
    pub chromium: String,
    pub firefox: String,
}

impl Manifests {
    pub fn new(host: &NativeMessagingHost) -> io::Result<Self> {
        if host.id.split('.').any(|part| {
            part.is_empty()
                || !part
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native messaging id must contain lowercase letters, digits, underscores, and single dots between nonempty components",
            ));
        }
        for origin in &host.allowed_origins {
            let valid = origin
                .strip_prefix("chrome-extension://")
                .and_then(|value| value.strip_suffix('/'))
                .is_some_and(|id| {
                    id.len() == 32 && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
                });
            if !valid {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "native messaging allowed_origins must be exact chrome-extension://<32-character extension id>/ origins",
                ));
            }
        }
        if host.allowed_extensions.iter().any(|id| {
            id.is_empty()
                || id
                    .chars()
                    .any(|ch| ch.is_whitespace() || ch.is_control() || ch == '*')
        }) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "native messaging allowed_extensions must contain explicit Firefox add-on ids",
            ));
        }
        let executable = std::path::absolute(&host.executable)?;
        if !executable.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "native messaging executable is not a file: {}",
                    executable.display()
                ),
            ));
        }
        let executable = executable.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "native messaging executable path must be valid Unicode",
            )
        })?;
        let manifest = |key: &str, allowed: &[String]| -> io::Result<String> {
            Ok(serde_json::to_string_pretty(&serde_json::json!({
                "name": host.id,
                "description": host.name,
                "path": executable,
                "type": "stdio",
                key: allowed,
            }))?)
        };
        Ok(Self {
            chromium: manifest("allowed_origins", &host.allowed_origins)?,
            firefox: manifest("allowed_extensions", &host.allowed_extensions)?,
        })
    }
}

pub(super) fn write_manifest(path: &Path, manifest: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, manifest)
}
