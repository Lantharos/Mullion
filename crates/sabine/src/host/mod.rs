#[cfg(target_os = "linux")]
use std::path::Path;
use std::path::PathBuf;

#[cfg(any(target_os = "windows", target_os = "macos"))]
mod desktop_wait;
mod process;
mod process_tree;

pub use process::{SabineProcess, SabineProcessHandle, WindowId};
pub(crate) use process_tree::{
    ManagedChild, prepare_child_command, prepare_detachable_child_command,
};
#[cfg(debug_assertions)]
pub(crate) use sabine_host::ensure_host;

#[cfg(not(debug_assertions))]
pub(crate) fn ensure_host(runtime_dir: &std::path::Path) -> Result<std::path::PathBuf, String> {
    sabine_host::available_host(runtime_dir).ok_or_else(||
        "The shared Sabine host is missing. Repair or update the Sabine installation before launching this app.".to_string())
}

pub(crate) fn browser_profile_dir(profile_key: &str) -> PathBuf {
    user_cache_home()
        .join("sabine")
        .join("profiles")
        .join(format!("{:016x}", stable_hash(&[profile_key])))
        .join("profile")
}

#[cfg(target_os = "linux")]
pub(crate) fn ld_library_path(release_dir: &Path) -> String {
    let existing = std::env::var("LD_LIBRARY_PATH").unwrap_or_default();
    if existing.is_empty() {
        release_dir.display().to_string()
    } else {
        format!("{}:{existing}", release_dir.display())
    }
}

fn user_cache_home() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(path) = std::env::var_os("LOCALAPPDATA").filter(|path| !path.is_empty()) {
        return PathBuf::from(path);
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME").filter(|path| !path.is_empty()) {
        return PathBuf::from(home).join("Library/Caches");
    }
    std::env::var_os("XDG_CACHE_HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .unwrap_or_else(std::env::temp_dir)
}

fn stable_hash(parts: &[&str]) -> u64 {
    let mut hash = 0xcbf29ce484222325u64;
    for part in parts {
        for byte in part.as_bytes() {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= 0xff;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}
