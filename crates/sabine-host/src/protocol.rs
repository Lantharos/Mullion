use std::{
    io::Read,
    path::Path,
    process::Stdio,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const HOST_PROTOCOL_VERSION: &str = "2";

pub fn validate_host_protocol(host: &Path, runtime_dir: &Path) -> Result<(), String> {
    let probe = std::env::temp_dir().join(format!(
        "sabine-host-check-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|error| error.to_string())?
            .as_nanos()
    ));
    std::fs::create_dir(&probe).map_err(|error| error.to_string())?;
    let _cleanup = crate::TemporaryDirectory(probe.clone());
    let output_path = probe.join("protocol");
    let output = std::fs::File::create(&output_path).map_err(|error| error.to_string())?;
    let mut command = sabine_runtime::background_command(host);
    let binary_dir = crate::runtime_binary_directory(runtime_dir);
    command
        .arg("--sabine-host-protocol")
        .arg("--sabine-runtime-smoke-test")
        .arg(format!(
            "--root-cache-path={}",
            probe.join("profile").display()
        ))
        .current_dir(&binary_dir)
        .stdin(Stdio::null())
        .stdout(output)
        .stderr(Stdio::null());
    crate::apply_runtime_resource_args(&mut command, runtime_dir);
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    {
        let variable = if cfg!(target_os = "windows") {
            "PATH"
        } else {
            "LD_LIBRARY_PATH"
        };
        let mut paths = vec![binary_dir];
        if let Some(existing) = std::env::var_os(variable) {
            paths.extend(std::env::split_paths(&existing));
        }
        command.env(
            variable,
            std::env::join_paths(paths).map_err(|error| error.to_string())?,
        );
    }
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not check Sabine host {}: {error}", host.display()))?;
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                let mut version = String::new();
                if let Ok(stdout) = std::fs::File::open(&output_path) {
                    let _ = stdout.take(64).read_to_string(&mut version);
                }
                if status.success() && version.trim() == HOST_PROTOCOL_VERSION {
                    return Ok(());
                }
                break;
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break;
            }
        }
    }
    Err(format!(
        "Sabine host {} uses an incompatible native protocol. Repair or update the shared Sabine installation before launching this app.",
        host.display()
    ))
}
