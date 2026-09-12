use std::{env, path::PathBuf};

fn main() {
    println!("cargo:rerun-if-changed=native/msi_actions.cc");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("sabine-msi.dll");
    let compiler = cc::Build::new().cpp(true).static_crt(true).get_compiler();
    let mut command = compiler.to_command();
    command.current_dir(env::var_os("OUT_DIR").unwrap());
    if compiler.is_like_msvc() {
        command.args(["/LD", "/MT", "/EHsc", "/std:c++17", "/O2"]);
        command.arg(format!("/Fe{}", output.display()));
    } else {
        command.args(["-shared", "-static", "-std=c++17", "-O2", "-o"]);
        command.arg(&output);
    }
    command.arg(
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("native/msi_actions.cc"),
    );
    if compiler.is_like_msvc() {
        command.args(["msi.lib", "user32.lib"]);
    } else {
        command.args(["-lmsi", "-luser32"]);
    }
    assert!(
        command
            .status()
            .expect("could not run the Windows C++ compiler")
            .success(),
        "could not build MSI setup actions"
    );
}
