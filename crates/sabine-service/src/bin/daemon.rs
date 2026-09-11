#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    if let Err(error) = sabine_service::run_daemon() {
        sabine_runtime::report_error("daemon", error);
        std::process::exit(1);
    }
}
