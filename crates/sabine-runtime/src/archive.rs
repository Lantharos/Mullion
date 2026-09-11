use std::{
    io::{self, Read},
    path::{Component, Path},
};

pub(crate) fn extract_tar(input: impl Read, destination: &Path) -> io::Result<()> {
    extract(input, destination, true)
}

/// Extracts a tar stream while confining paths and links to its destination.
/// Callers must provide a private staging directory, then validate the payload
/// before activating it.
pub fn extract_tar_archive(input: impl Read, destination: &Path) -> io::Result<()> {
    extract(input, destination, false)
}

fn extract(input: impl Read, destination: &Path, version_root: bool) -> io::Result<()> {
    std::fs::create_dir_all(destination)?;
    let mut archive = tar::Archive::new(input);
    let mut size = 0u64;
    for (index, entry) in archive.entries()?.enumerate() {
        if index >= 100_000 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive has too many entries",
            ));
        }
        let mut entry = entry?;
        let path = entry.path()?.into_owned();
        if !safe_path(&path) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive contains an unsafe path",
            ));
        }
        let kind = entry.header().entry_type();
        if !(kind.is_file() || kind.is_dir() || kind.is_symlink() || kind.is_hard_link()) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive contains an unsupported entry type",
            ));
        }
        if let Some(target) = entry.link_name()? {
            let base = if kind.is_symlink() {
                path.parent().unwrap_or(Path::new(""))
            } else {
                Path::new("")
            };
            if !safe_link(base, &target, &path, version_root) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "archive link escapes its installation",
                ));
            }
        }
        size = size
            .checked_add(entry.size())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "archive size overflow"))?;
        if size > 16 * 1024 * 1024 * 1024 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive exceeds 16 GiB",
            ));
        }
        if !entry.unpack_in(destination)? {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "archive entry could not be confined to its installation",
            ));
        }
    }
    Ok(())
}

fn safe_path(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.to_string_lossy().contains(['\\', ':'])
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir))
}

fn safe_link(base: &Path, target: &Path, entry: &Path, version_root: bool) -> bool {
    if target.as_os_str().is_empty() || target.to_string_lossy().contains(['\\', ':']) {
        return false;
    }
    let mut parts = base
        .components()
        .filter_map(|part| match part {
            Component::Normal(part) => Some(part.to_os_string()),
            _ => None,
        })
        .collect::<Vec<_>>();
    for part in target.components() {
        match part {
            Component::Normal(part) => parts.push(part.to_os_string()),
            Component::CurDir => {}
            Component::ParentDir if !parts.is_empty() => {
                parts.pop();
            }
            _ => return false,
        }
    }
    if !version_root {
        return true;
    }
    let root = entry.components().find_map(|part| match part {
        Component::Normal(part) => Some(part),
        _ => None,
    });
    parts.first().map(|part| part.as_os_str()) == root
}
