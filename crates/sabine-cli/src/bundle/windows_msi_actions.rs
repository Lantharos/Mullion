use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn stage(root: &Path, architecture: &str) -> Result<PathBuf, String> {
    let directory = root.join("msi-actions");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let dll = directory.join("sabine-msi.dll");
    fs::write(
        directory.join("actions.cc"),
        include_str!("../../native/msi_actions.cc"),
    )
    .map_err(|error| error.to_string())?;
    fs::write(
        directory.join("CMakeLists.txt"),
        r#"cmake_minimum_required(VERSION 3.21)
project(sabine_msi_actions LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 17)
set(CMAKE_MSVC_RUNTIME_LIBRARY MultiThreaded)
add_library(sabine-msi SHARED actions.cc)
target_link_libraries(sabine-msi msi user32)
set_target_properties(sabine-msi PROPERTIES
  RUNTIME_OUTPUT_DIRECTORY "${CMAKE_CURRENT_SOURCE_DIR}"
  RUNTIME_OUTPUT_DIRECTORY_RELEASE "${CMAKE_CURRENT_SOURCE_DIR}")
"#,
    )
    .map_err(|error| error.to_string())?;
    #[cfg(windows)]
    if matches!(
        (architecture, std::env::consts::ARCH),
        ("x64", "x86_64") | ("ARM64", "aarch64")
    ) {
        fs::write(
            &dll,
            include_bytes!(concat!(env!("OUT_DIR"), "/sabine-msi.dll")),
        )
        .map_err(|error| error.to_string())?;
        return Ok(dll);
    }
    if cfg!(windows) {
        let build = directory.join("build");
        let commands: [Vec<std::ffi::OsString>; 2] = [
            vec![
                "-S".into(),
                directory.as_os_str().into(),
                "-B".into(),
                build.as_os_str().into(),
                "-A".into(),
                architecture.into(),
            ],
            vec![
                "--build".into(),
                build.as_os_str().into(),
                "--config".into(),
                "Release".into(),
            ],
        ];
        for command in commands {
            let status = std::process::Command::new("cmake")
                .args(command)
                .status()
                .map_err(|error| error.to_string())?;
            if !status.success() {
                return Err("could not build MSI actions for the app's architecture".into());
            }
        }
    }
    Ok(dll)
}
