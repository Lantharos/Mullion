use crate::window::config::SabineWindowConfig;
use sabine_service::{SabineService, ServiceError, prepare_machine_with_progress};
use std::io::Write;

pub(crate) const INSTALL_ARG: &str = "--sabine-install";
pub(crate) const UNINSTALL_ARG: &str = "--sabine-uninstall";

pub(crate) fn requested(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg == INSTALL_ARG || arg == UNINSTALL_ARG)
}

pub(crate) fn run(config: &SabineWindowConfig, args: &[String]) -> ! {
    let cancelled = || {
        option(args, "--sabine-install-cancel")
            .is_some_and(|path| std::path::Path::new(path).exists())
    };
    let result = if args.iter().any(|arg| arg == UNINSTALL_ARG) {
        unregister(config)
    } else {
        prepare(config, option(args, "--sabine-install-to"), cancelled)
    };
    match result {
        Ok(()) => std::process::exit(0),
        Err(error) => {
            if cancelled() {
                println!("Installation cancelled.");
                std::process::exit(1602);
            }
            sabine_runtime::report_error("installer", &error);
            println!("{error}");
            println!(
                "Details: {}",
                sabine_runtime::diagnostic_path("installer").display()
            );
            std::process::exit(1);
        }
    }
}

fn option<'a>(args: &'a [String], name: &str) -> Option<&'a str> {
    args.iter()
        .position(|arg| arg == name)
        .and_then(|index| args.get(index + 1))
        .map(String::as_str)
}

fn prepare(
    config: &SabineWindowConfig,
    destination: Option<&str>,
    cancelled: impl Fn() -> bool,
) -> Result<(), String> {
    config.validate().map_err(|error| error.to_string())?;
    let manifest = super::app_manifest(config).ok_or("The app has no installation identity")?;
    let report = prepare_machine_with_progress(config.runtime.clone(), None, |progress| {
        if cancelled() {
            println!("Installation cancelled.");
            std::process::exit(1602);
        }
        if let Some(fraction) = progress.fraction {
            println!(
                "{:>3}% {}",
                (fraction * 100.0).round() as u32,
                progress.message
            );
        } else {
            println!("{}", progress.message);
        }
        let _ = std::io::stdout().flush();
    })
    .map_err(|error| error.to_string())?;
    if !report.daemon_running {
        return Err(
            "The Sabine background service could not start. Retry setup to repair it.".into(),
        );
    }
    let runtime =
        sabine_runtime::resolve_runtime(&config.runtime).map_err(|error| error.to_string())?;
    let host = sabine_host::available_host(runtime.location.path())
        .ok_or("The Sabine native host is missing. Retry setup to repair it.")?;
    sabine_runtime::prepare_runtime_assets(runtime.location.path())
        .map_err(|error| error.to_string())?;
    sabine_host::validate_host_protocol(&host, runtime.location.path())?;
    if let Some(destination) = destination {
        println!("Installing application files...");
        let source = manifest
            .executable
            .parent()
            .ok_or("The installer payload has no directory")?
            .to_path_buf();
        SabineService::default()
            .install_app_payload(
                &source,
                std::path::Path::new(destination),
                manifest,
                cancelled,
            )
            .map_err(|error| error.to_string())?;
    } else {
        SabineService::default()
            .register(manifest)
            .map_err(|error| error.to_string())?;
    }
    println!("Installation is ready.");
    Ok(())
}

fn unregister(config: &SabineWindowConfig) -> Result<(), String> {
    let id = config
        .app_id
        .as_deref()
        .ok_or("The app has no installation identity")?;
    match SabineService::default().unregister(id) {
        Ok(_) | Err(ServiceError::AppNotFound(_)) => {
            println!("Application registration removed.");
            Ok(())
        }
        Err(error) => Err(error.to_string()),
    }
}
