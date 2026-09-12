use std::{io, path::Path};

pub fn prepare_runtime_assets(runtime_dir: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    {
        let assets = if cfg!(windows) {
            &[
                "icudtl.dat",
                "chrome_100_percent.pak",
                "chrome_200_percent.pak",
                "resources.pak",
            ][..]
        } else {
            &["icudtl.dat"][..]
        };
        for asset in assets {
            link_runtime_asset(runtime_dir, asset)?;
        }
    }
    #[cfg(windows)]
    crate::prepare_sandbox_access(runtime_dir, true).map_err(io::Error::other)?;
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = runtime_dir;
    Ok(())
}

#[cfg(any(target_os = "linux", windows))]
fn link_runtime_asset(runtime_dir: &Path, asset: &str) -> io::Result<()> {
    let target = runtime_dir.join("Release").join(asset);
    if !target.is_file() {
        let source = runtime_dir.join("Resources").join(asset);
        match std::fs::hard_link(&source, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists && target.is_file() => {}
            Err(error) => {
                return Err(io::Error::new(
                    error.kind(),
                    format!(
                        "could not prepare CEF resource at {}: {error}",
                        target.display()
                    ),
                ));
            }
        }
    }
    Ok(())
}
