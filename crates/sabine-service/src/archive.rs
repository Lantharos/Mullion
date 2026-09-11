use std::{
    fs::{self, File},
    io::{self, Read, Seek},
    path::{Component, Path},
};

use crate::{ServiceError, ServiceResult};

const MAX_EXPANDED_SIZE: u64 = 16 * 1024 * 1024 * 1024;
const MAX_ENTRIES: usize = 100_000;

pub(crate) fn extract(archive: &Path, destination: &Path) -> ServiceResult<()> {
    extract_inner(archive, destination).map_err(|error| {
        ServiceError::Update(format!("could not extract {}: {error}", archive.display()))
    })
}

fn extract_inner(archive: &Path, destination: &Path) -> io::Result<()> {
    fs::create_dir_all(destination)?;
    if fs::read_dir(destination)?.next().is_some() {
        return Err(invalid(
            "archive extraction requires an empty staging directory",
        ));
    }
    let mut input = File::open(archive)?;
    let mut signature = [0u8; 4];
    input.read_exact(&mut signature)?;
    input.rewind()?;
    if signature.starts_with(b"PK") {
        extract_zip(input, destination)
    } else if signature.starts_with(&[0x1f, 0x8b]) {
        sabine_runtime::extract_tar_archive(flate2::read::MultiGzDecoder::new(input), destination)
    } else {
        sabine_runtime::extract_tar_archive(input, destination)
    }
}

fn extract_zip(input: File, destination: &Path) -> io::Result<()> {
    let destination = destination.canonicalize()?;
    let mut archive = zip::ZipArchive::new(input)?;
    if archive.len() > MAX_ENTRIES {
        return Err(invalid("archive has too many entries"));
    }
    let mut total = 0u64;
    let mut links = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let path = Path::new(entry.name()).to_path_buf();
        if !safe_path(&path) {
            return Err(invalid("archive contains an unsafe path"));
        }
        let mode = entry.unix_mode().unwrap_or(0o644);
        if !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000 | 0o120000) {
            return Err(invalid("archive contains an unsupported entry type"));
        }
        let size = entry.size();
        total = total
            .checked_add(size)
            .ok_or_else(|| invalid("archive size overflow"))?;
        if total > MAX_EXPANDED_SIZE {
            return Err(invalid("archive exceeds 16 GiB"));
        }
        let output = destination.join(&path);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }
        fs::create_dir_all(
            output
                .parent()
                .ok_or_else(|| invalid("entry has no parent"))?,
        )?;
        if entry.is_symlink() {
            if size > 4096 {
                return Err(invalid("archive symlink target is too long"));
            }
            let mut target = String::new();
            (&mut entry).take(size + 1).read_to_string(&mut target)?;
            if target.len() as u64 != size || !safe_target(&path, Path::new(&target)) {
                return Err(invalid("archive symlink escapes its installation"));
            }
            links.push((output, target));
        } else {
            let mut file = File::create_new(&output)?;
            if io::copy(&mut (&mut entry).take(size + 1), &mut file)? != size {
                return Err(invalid("archive entry size does not match its metadata"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
    }
    for (path, target) in &links {
        #[cfg(unix)]
        std::os::unix::fs::symlink(target, path)?;
        #[cfg(windows)]
        if path.parent().unwrap().join(target).is_dir() {
            std::os::windows::fs::symlink_dir(target, path)?;
        } else {
            std::os::windows::fs::symlink_file(target, path)?;
        }
    }
    for (path, _) in links {
        if !path.canonicalize()?.starts_with(&destination) {
            return Err(invalid("archive symlink resolves outside its installation"));
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

fn safe_target(path: &Path, target: &Path) -> bool {
    if target.as_os_str().is_empty() || target.to_string_lossy().contains(['\\', ':']) {
        return false;
    }
    let mut depth = path
        .parent()
        .unwrap_or(Path::new(""))
        .components()
        .filter(|part| matches!(part, Component::Normal(_)))
        .count();
    for part in target.components() {
        match part {
            Component::Normal(_) => depth += 1,
            Component::CurDir => {}
            Component::ParentDir if depth > 0 => depth -= 1,
            _ => return false,
        }
    }
    true
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
