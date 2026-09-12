pub fn info_plist(
    id: &str,
    name: &str,
    version: &str,
    executable: &str,
    has_icon: bool,
) -> Result<String, String> {
    let version = semver::Version::parse(version).map_err(|error| error.to_string())?;
    let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let icon = if has_icon {
        "<key>CFBundleIconFile</key><string>app.icns</string>"
    } else {
        ""
    };
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
