use std::{io, path::Path};

pub fn prepare_runtime_assets(runtime_dir: &Path) -> io::Result<()> {
    #[cfg(any(target_os = "linux", windows))]
    {
        let target = runtime_dir.join("Release/icudtl.dat");
        if !target.is_file() {
            let source = runtime_dir.join("Resources/icudtl.dat");
            match std::fs::hard_link(&source, &target) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists && target.is_file() => {}
                Err(error) => {
                    return Err(io::Error::new(
                        error.kind(),
                        format!(
                            "could not prepare CEF ICU data at {}: {error}",
                            target.display()
                        ),
                    ));
                }
            }
        }
    }
    #[cfg(windows)]
    crate::prepare_sandbox_access(runtime_dir, true).map_err(io::Error::other)?;
    #[cfg(not(any(target_os = "linux", windows)))]
    let _ = runtime_dir;
    Ok(())
}
