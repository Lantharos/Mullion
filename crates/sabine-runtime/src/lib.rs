#[cfg(all(target_os = "macos", not(target_arch = "aarch64")))]
compile_error!("Sabine requires Apple Silicon on macOS");

mod archive;
mod assets;
mod detect;
mod diagnostics;
mod directory_install;
mod download;
mod error;
mod file_lock;
mod host;
mod install;
mod lease;
mod paths;
mod process;
mod resolve;
#[cfg(windows)]
mod sandbox_windows;
mod types;
mod version;
#[cfg(windows)]
pub use sandbox_windows::prepare_sandbox_access;

pub(crate) const MIN_CEF_MAJOR: &str = "151";

pub use archive::extract_tar_archive;
pub use assets::prepare_runtime_assets;
pub use directory_install::{install_directory, recover_directory_installs};
pub use download::transfer::{DownloadProgress, download_file as download_file_with_progress};
pub use download::{DEFAULT_CEF_INDEX_URL, latest_install_plan};
pub use error::RuntimeError;
pub use file_lock::FileLock;
pub use install::{
    install_user_runtime, install_user_runtime_with_progress, prune_user_runtimes,
    quarantine_user_runtime, remove_user_runtime_version, update_user_runtime_with_progress,
};
pub use lease::RuntimeLease;
#[cfg(target_os = "macos")]
pub use paths::runtime_execution_path;
pub use paths::{
    bundled_runtime_path, runtime_version_path, system_runtime_path, user_runtime_path,
};
pub use process::{background_command, configure_background_command};
pub use resolve::{ensure_runtime, resolve_runtime};
pub use types::{
    RuntimeConfig, RuntimeInfo, RuntimeInstallPlan, RuntimeInstallProgress, RuntimeInstallStep,
    RuntimeLocation, RuntimeMode,
};

pub use detect::detect_runtime;
pub use diagnostics::{capture_diagnostics, diagnostic_path, record_diagnostic, report_error};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_mode_round_trips() {
        assert_eq!(
            RuntimeMode::SharedPreferred,
            RuntimeMode::parse("shared-preferred").unwrap()
        );
        assert_eq!(
            RuntimeMode::SystemRequired,
            RuntimeMode::parse("system-required").unwrap()
        );
        assert!(RuntimeMode::parse("invalid").is_none());
    }

    #[test]
    fn detect_runtime_skips_missing_dirs() {
        let config = RuntimeConfig::default();
        let runtimes = detect_runtime(&config);
        assert!(runtimes.is_empty() || runtimes.iter().all(|r| r.location.path().is_dir()));
    }

    #[test]
    fn version_checks_use_major_version() {
        assert!(crate::version::version_satisfies(
            "147.0.14+gabc+chromium-147.0.7727.138",
            "126"
        ));
        assert!(!crate::version::version_satisfies(
            "101.0.18+gabc+chromium-101.0.4951.67",
            "126"
        ));
    }
}
