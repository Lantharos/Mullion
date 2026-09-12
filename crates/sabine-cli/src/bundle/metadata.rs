use super::config::BundleApp;
use sabine_service::{AppArtifactKind, AppInstallMode, AppUpdateSource};

pub(super) fn runtime_manifest(
    app: &BundleApp,
    web_directory: &str,
    install_mode: AppInstallMode,
    package_kind: Option<AppArtifactKind>,
) -> Result<String, String> {
    let mut manifest = format!(
        "[app]\nid = \"{}\"\nname = \"{}\"\nversion = \"{}\"\n",
        quote(&app.id),
        quote(&app.name),
        quote(&app.version)
    );
    if !app.mime_types.is_empty() {
        manifest.push_str(&format!(
            "mime_types = {}\n",
            serde_json::to_string(&app.mime_types).map_err(|error| error.to_string())?
        ));
    }
    if let Some(web) = &app.web {
        manifest.push_str("\n[web]\n");
        if let Some((_, relative_entry)) = web.assets()? {
            let entry = std::path::Path::new(web_directory).join(relative_entry);
            manifest.push_str(&format!(
                "entry = \"{}\"\n",
                quote(&entry.display().to_string())
            ));
        } else if let Some(url) = &web.url {
            manifest.push_str(&format!("url = \"{}\"\n", quote(url)));
        }
        if !web.allowed_origins.is_empty() {
            let origins = web
                .allowed_origins
                .iter()
                .map(|origin| format!("\"{}\"", quote(origin)))
                .collect::<Vec<_>>()
                .join(", ");
            manifest.push_str(&format!("allowed_origins = [{origins}]\n"));
        }
    }
    if let Some(updates) = &app.updates {
        manifest.push_str("\n[updates]\n");
        match &updates.source {
            AppUpdateSource::Github { repository } => manifest.push_str(&format!(
                "provider = \"github\"\nrepository = \"{}\"\n",
                quote(repository)
            )),
            AppUpdateSource::Http { url } => {
                manifest.push_str(&format!("provider = \"http\"\nurl = \"{}\"\n", quote(url)))
            }
        }
        manifest.push_str(&format!(
            "channel = \"{}\"\npolicy = \"{}\"\ninstall_mode = \"{}\"\npublic_key = \"{}\"\n",
            quote(&updates.channel),
            match updates.policy {
                sabine_service::UpdatePolicy::Disabled => "disabled",
                sabine_service::UpdatePolicy::Notify => "notify",
                sabine_service::UpdatePolicy::Automatic => "automatic",
            },
            match install_mode {
                AppInstallMode::Managed => "managed",
                AppInstallMode::Package => "package",
                AppInstallMode::Store => "store",
            },
            quote(&updates.public_key)
        ));
        if let Some(kind) = package_kind {
            manifest.push_str(&format!("package_kind = \"{}\"\n", kind.config_value()));
        }
    }
    Ok(manifest)
}

pub(super) fn desktop_entry(app: &BundleApp, executable: &str, icon: Option<&str>) -> String {
    let icon = icon
        .map(|icon| format!("Icon={}\n", desktop_value(icon)))
        .unwrap_or_default();
    let mime_types = mime_type_line(&app.mime_types);
    format!(
        "[Desktop Entry]\nType=Application\nName={}\nExec={} %U\n{}{}Terminal=false\nCategories=Utility;\nStartupNotify=true\nStartupWMClass={}\n",
        desktop_value(&app.name),
        desktop_exec(executable),
        icon,
        mime_types,
        desktop_value(&app.id)
    )
}

pub(super) fn app_run(executable: &str) -> String {
    format!(
        "#!/bin/sh\nHERE=\"$(dirname \"$(readlink -f \"$0\")\")\"\nexec \"$HERE/usr/bin/{executable}\" \"$@\"\n"
    )
}

pub(super) fn windows_manifest(app: &BundleApp) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <assemblyIdentity version="{}" processorArchitecture="*" name="{}" type="win32"/>
  <description>{}</description>
  <dependency><dependentAssembly><assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" language="*"/></dependentAssembly></dependency>
</assembly>
"#,
        windows_version(&app.version),
        xml(&app.id),
        xml(&app.name)
    )
}

pub(super) fn shell_script(lines: &[&str]) -> String {
    format!("#!/bin/sh\nset -e\n{}\n", lines.join("\n"))
}

pub(super) fn sanitize_path(value: &str) -> String {
    value
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect()
}

fn quote(value: &str) -> String {
    let encoded = serde_json::to_string(value).expect("string serialization");
    encoded[1..encoded.len() - 1].to_string()
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn desktop_value(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}

fn mime_type_line(mime_types: &[String]) -> String {
    if mime_types.is_empty() {
        return String::new();
    }
    let values = mime_types
        .iter()
        .map(|mime_type| mime_type.trim())
        .filter(|mime_type| !mime_type.is_empty())
        .collect::<Vec<_>>();
    if values.is_empty() {
        String::new()
    } else {
        format!("MimeType={};\n", values.join(";"))
    }
}

fn desktop_exec(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('`', "\\`")
        .replace('$', "\\$")
        .replace('%', "%%");
    format!("\"{}\"", escaped.replace('\\', "\\\\"))
}

fn windows_version(version: &str) -> String {
    let version = semver::Version::parse(version).expect("validated app version");
    format!("{}.{}.{}.0", version.major, version.minor, version.patch)
}
