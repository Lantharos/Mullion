pub(crate) mod installer;
mod ui;

use crate::window::config::SabineWindowConfig;
use crate::{SabineError, SabineResult};
use sabine_runtime::{
    RuntimeConfig, RuntimeMode, background_command, configure_background_command, resolve_runtime,
};
use sabine_service::{AppManifest, SabineService, prepare_machine_with_progress};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const CONFIRM_UPDATE_ARG: &str = "--sabine-confirm-update";

pub(crate) const NOTICE_ARG: &str = "--sabine-notice";

pub(crate) fn show_failure(title: &str, error: &dyn std::fmt::Display) {
    sabine_runtime::report_error("startup", error);
    let message = format!(
        "{error}\n\nDetails: {}",
        sabine_runtime::diagnostic_path("startup").display()
    );
    if let Ok(executable) = std::env::current_exe() {
        let _ = background_command(executable)
            .args([NOTICE_ARG, title, &message])
            .status();
    }
}

pub(crate) const BOOTSTRAP_ARG: &str = "--sabine-bootstrap";

pub(crate) fn run_from_args(args: &[String]) -> bool {
    if let Some(index) = args.iter().position(|arg| arg == CONFIRM_UPDATE_ARG) {
        let accepted = args
            .get(index + 1)
            .zip(args.get(index + 2))
            .is_some_and(|(title, version)| ui::confirm_update(title, version).unwrap_or(false));
        std::process::exit(if accepted { 0 } else { 2 });
    }
    if let Some(index) = args.iter().position(|arg| arg == NOTICE_ARG) {
        if let (Some(title), Some(message)) = (args.get(index + 1), args.get(index + 2))
            && let Err(error) = ui::show_notice(title, message)
        {
            sabine_runtime::report_error("startup", error);
        }
        return true;
    }
    let Some(index) = args.iter().position(|arg| arg == BOOTSTRAP_ARG) else {
        return false;
    };
    let Some(path) = args.get(index + 1).map(PathBuf::from) else {
        eprintln!("missing Sabine bootstrap config");
        std::process::exit(1);
    };
    let (config, register) = match read_bootstrap(path) {
        Ok(value) => value,
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    };
    let result = ui::run_progress_window("Preparing Sabine", move |state, proxy| {
        let result = prepare_machine_with_progress(config, register, |progress| {
            ui::set_progress(&state, &proxy, progress.message, progress.fraction);
        });
        if let Err(error) = &result {
            sabine_runtime::report_error("setup", error);
        }
        ui::finish(
            &state,
            &proxy,
            result.map(|_| ()).map_err(|error| error.to_string()),
        );
    });
    match result {
        Ok(ui::ProgressOutcome::Complete) => true,
        Ok(ui::ProgressOutcome::Cancelled) => std::process::exit(2),
        Ok(ui::ProgressOutcome::Failed) => std::process::exit(3),
        Err(error) => {
            sabine_runtime::report_error("setup", error);
            std::process::exit(1);
        }
    }
}

pub(crate) fn prepare(config: &SabineWindowConfig) -> SabineResult<()> {
    let register = app_manifest(config);

    if resolve_runtime(&config.runtime).is_ok() {
        let report =
            match sabine_service::adopt_with_runtime(config.runtime.clone(), register.clone()) {
                Ok(report) => report,
                Err(error @ sabine_service::ServiceError::IncompatibleApp { .. }) => {
                    let message = error.to_string();
                    return Err(SabineError::CreationFailed { message });
                }
                Err(error) => {
                    return Err(SabineError::CreationFailed {
                        message: format!("failed to register with Sabine service: {error}"),
                    });
                }
            };
        if std::env::var_os("SABINE_TRACE").is_some() {
            eprintln!(
                "sabine-service ready runtime={} daemon={} login_autostart={}",
                report.runtime_version, report.daemon_running, report.login_autostart
            );
        }
        relaunch_managed_update(config);
        offer_pending_update(config);
        return Ok(());
    }

    run_bootstrap_install(config, register)?;
    relaunch_managed_update(config);
    offer_pending_update(config);
    Ok(())
}

fn relaunch_managed_update(config: &SabineWindowConfig) {
    let Some(id) = config.app_id.as_deref() else {
        return;
    };
    let Ok(registered) = SabineService::default().app(id) else {
        return;
    };
    let Ok(current) = std::env::current_exe() else {
        return;
    };
    let desired = registered.manifest.executable;
    let current = current.canonicalize().unwrap_or(current);
    let desired = desired.canonicalize().unwrap_or(desired);
    if current == desired || !desired.is_file() {
        return;
    }
    let mut command = background_command(desired);
    let launched = command
        .args(std::env::args_os().skip(1))
        .stdin(Stdio::null())
        .spawn()
        .is_ok();
    if launched {
        std::process::exit(0);
    }
}

fn offer_pending_update(config: &SabineWindowConfig) {
    let Some(id) = config.app_id.as_deref() else {
        return;
    };
    let service = SabineService::default();
    let Ok(Some(update)) = service.pending_app_update(id) else {
        return;
    };
    if !update.ready_for_prompt() {
        return;
    }
    let accepted = std::env::current_exe().ok().is_some_and(|executable| {
        background_command(executable)
            .args([CONFIRM_UPDATE_ARG, &config.title, &update.version])
            .status()
            .is_ok_and(|status| status.success())
    });
    if !accepted {
        let _ = service.defer_pending_app_update(id);
        return;
    }
    let Ok(service_executable) = sabine_service::resolve_service_executable() else {
        return;
    };
    let executable = std::env::var_os("APPIMAGE")
        .map(PathBuf::from)
        .or_else(|| std::env::current_exe().ok());
    let Some(executable) = executable else {
        return;
    };
    let mut command = background_command(service_executable);
    let spawned = command
        .arg("apply-update")
        .arg(id)
        .arg("--wait-pid")
        .arg(std::process::id().to_string())
        .arg("--relaunch")
        .arg(executable)
        .stdin(Stdio::null())
        .spawn()
        .is_ok();
    if spawned {
        std::process::exit(0);
    }
}

fn run_bootstrap_install(
    config: &SabineWindowConfig,
    register: Option<AppManifest>,
) -> SabineResult<()> {
    let config_path = bootstrap_config_path();
    write_bootstrap(&config_path, &config.runtime, register.as_ref()).map_err(|error| {
        SabineError::CreationFailed {
            message: format!("failed to prepare Sabine bootstrap: {error}"),
        }
    })?;
    let executable = std::env::current_exe().map_err(|error| SabineError::CreationFailed {
        message: format!("failed to locate app executable: {error}"),
    })?;
    let mut command = Command::new(executable);
    configure_background_command(&mut command);
    let status = command
        .arg(BOOTSTRAP_ARG)
        .arg(&config_path)
        .stdin(Stdio::null())
        .status()
        .map_err(|error| SabineError::CreationFailed {
            message: format!("failed to launch Sabine bootstrap: {error}"),
        })?;
    let _ = std::fs::remove_file(config_path);
    match status.code() {
        Some(0) => Ok(()),
        Some(2) => Err(SabineError::SetupCancelled),
        Some(3) => Err(SabineError::SetupFailed),
        _ => Err(SabineError::CreationFailed {
            message: format!(
                "Sabine setup stopped unexpectedly ({status}); details: {}",
                sabine_runtime::diagnostic_path("setup").display()
            ),
        }),
    }
}

pub(crate) fn app_manifest(config: &SabineWindowConfig) -> Option<AppManifest> {
    let id = config.app_id.as_ref()?;
    let executable = std::env::current_exe().ok()?;
    Some(AppManifest {
        id: id.clone(),
        name: config.title.clone(),
        version: config
            .app_version
            .clone()
            .unwrap_or_else(|| "0.0.0".to_string()),
        executable,
        args: Vec::new(),
        update: config.app_update.clone(),
        sabine: sabine_service::SabineVersion::current(),
    })
}

fn write_bootstrap(
    path: &Path,
    config: &RuntimeConfig,
    register: Option<&AppManifest>,
) -> std::io::Result<()> {
    let mode = match config.mode {
        RuntimeMode::SystemRequired => "system-required",
        RuntimeMode::SystemPreferred => "system-preferred",
        RuntimeMode::SharedPreferred => "shared-preferred",
        RuntimeMode::Bundled => "bundled",
    };
    let mut body = serde_json::json!({
        "mode": mode,
        "index_url": config.index_url,
        "allow_user_install": config.allow_user_install,
        "allow_bundled": config.allow_bundled,
    });
    if let Some(dir) = &config.bundled_dir {
        body["bundled_dir"] = dir.display().to_string().into();
    }
    if let Some(manifest) = register {
        body["register"] = serde_json::to_value(manifest).expect("app manifest is serializable");
    }
    std::fs::write(
        path,
        serde_json::to_vec(&body).expect("bootstrap config is serializable"),
    )
}

fn read_bootstrap(path: PathBuf) -> Result<(RuntimeConfig, Option<AppManifest>), String> {
    let value = serde_json::from_slice::<Value>(&std::fs::read(&path).map_err(|e| e.to_string())?)
        .map_err(|error| error.to_string())?;
    let _ = std::fs::remove_file(path);
    let config = RuntimeConfig {
        mode: value
            .get("mode")
            .and_then(Value::as_str)
            .and_then(RuntimeMode::parse)
            .unwrap_or(RuntimeMode::SharedPreferred),
        index_url: value
            .get("index_url")
            .and_then(Value::as_str)
            .map(ToString::to_string),
        allow_user_install: value
            .get("allow_user_install")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        allow_bundled: value
            .get("allow_bundled")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        bundled_dir: value
            .get("bundled_dir")
            .and_then(Value::as_str)
            .map(PathBuf::from),
    };
    let register = value
        .get("register")
        .cloned()
        .map(serde_json::from_value::<AppManifest>)
        .transpose()
        .map_err(|error| format!("invalid app registration: {error}"))?;
    Ok((config, register))
}

fn bootstrap_config_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!(
        "sabine-bootstrap-{}-{nonce}.json",
        std::process::id()
    ))
}
