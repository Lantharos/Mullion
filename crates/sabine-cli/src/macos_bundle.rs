use std::{fs::File, io::Read, path::Path};

pub fn validate_executable(path: &Path) -> Result<(), String> {
    let mut header = [0u8; 32];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| format!("could not inspect {}: {error}", path.display()))?;
    let magic = u32::from_le_bytes(header[0..4].try_into().unwrap());
    let cpu = u32::from_le_bytes(header[4..8].try_into().unwrap());
    let kind = u32::from_le_bytes(header[12..16].try_into().unwrap());
    if magic != 0xfeed_facf || cpu != 0x0100_000c || kind != 2 {
        return Err(format!(
            "macOS bundles require an Apple Silicon Mach-O executable; rebuild {} for aarch64-apple-darwin",
            path.display()
        ));
    }
    Ok(())
}

pub fn info_plist(
    id: &str,
    name: &str,
    version: &str,
    executable: &str,
    has_icon: bool,
    mime_types: &[String],
) -> Result<String, String> {
    crate::desktop_types::validate(mime_types)?;
    let version = semver::Version::parse(version).map_err(|error| error.to_string())?;
    let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let icon = if has_icon {
        "<key>CFBundleIconFile</key><string>app.icns</string>"
    } else {
        ""
    };
    let mut types = String::new();
    let schemes = crate::desktop_types::schemes(mime_types)
        .map(|scheme| format!("<string>{}</string>", xml(scheme)))
        .collect::<String>();
    if !schemes.is_empty() {
        types.push_str(&format!("<key>CFBundleURLTypes</key><array><dict><key>CFBundleURLName</key><string>{}</string><key>CFBundleTypeRole</key><string>Editor</string><key>CFBundleURLSchemes</key><array>{schemes}</array></dict></array>\n", xml(id)));
    }
    let documents = mime_types.iter().filter(|mime| !mime.starts_with("x-scheme-handler/")).map(|mime| format!("<dict><key>CFBundleTypeName</key><string>{0}</string><key>CFBundleTypeRole</key><string>Viewer</string><key>LSHandlerRank</key><string>Alternate</string><key>CFBundleTypeMIMETypes</key><array><string>{0}</string></array></dict>", xml(mime))).collect::<String>();
    if !documents.is_empty() {
        types.push_str(&format!(
            "<key>CFBundleDocumentTypes</key><array>{documents}</array>\n"
        ));
    }
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleIdentifier</key><string>{}</string>
<key>CFBundleName</key><string>{}</string>
<key>CFBundleDisplayName</key><string>{}</string>
<key>CFBundleExecutable</key><string>{}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
{icon}
{types}
</dict></plist>
"#,
        xml(id),
        xml(name),
        xml(name),
        xml(executable)
    ))
}

pub fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
