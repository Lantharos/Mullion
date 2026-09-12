use super::{config::BundleApp, stage::StagedBundle};
use std::{fs, io::Read, path::Path, process::Command};

const CEF_DEB_DEPENDENCIES: &str = "libgtk-3-0t64 | libgtk-3-0, libnss3, libnspr4, libasound2t64 | libasound2, libcups2t64 | libcups2, libxcomposite1, libxdamage1, libxrandr2, libgbm1, libxkbcommon0, libxkbcommon-x11-0, libudev1, libwayland-client0";

pub(super) fn architecture(binary: &Path) -> Result<(&'static str, &'static str), String> {
    let mut header = [0; 20];
    fs::File::open(binary)
        .and_then(|mut file| file.read_exact(&mut header))
        .map_err(|error| error.to_string())?;
    if &header[..4] != b"\x7fELF" || header[4] != 2 || header[5] != 1 {
        return Err("Linux packages require a 64-bit little-endian ELF executable".into());
    }
    match u16::from_le_bytes([header[18], header[19]]) {
        62 => Ok(("amd64", "x86_64")),
        183 => Ok(("arm64", "aarch64")),
        _ => Err("Linux packages support x86_64 and aarch64".into()),
    }
}

pub(super) fn deb_control(
    app: &BundleApp,
    binary: &Path,
    installed_size_kb: u64,
    dependencies: &str,
) -> Result<String, String> {
    let maintainer = app
        .maintainer
        .as_deref()
        .filter(|value| {
            value.rsplit_once('<').is_some_and(|(name, email)| {
                !name.trim().is_empty()
                    && email.ends_with('>')
                    && email.contains('@')
                    && !email.chars().any(char::is_whitespace)
            })
        })
        .ok_or(
            "Debian packaging requires app.maintainer or a Cargo author in the form Name <email>",
        )?;
    let version = package_version(&app.version)?;
    let (architecture, _) = architecture(binary)?;
    Ok(format!(
        "Package: {}\nVersion: {version}\nSection: utils\nPriority: optional\nArchitecture: {architecture}\nMaintainer: {maintainer}\nInstalled-Size: {}\nDepends: {dependencies}\nDescription: {}\n",
        app.id,
        installed_size_kb.max(1),
        app.name
    ))
}

pub(super) fn deb_dependencies(staged: &StagedBundle) -> Result<String, String> {
    if !crate::commands::command_exists("dpkg-shlibdeps") {
        return Err("Debian packaging requires dpkg-dev on the target Debian/Ubuntu build environment to calculate library dependencies".into());
    }
    let debian = staged.root.join("debian");
    fs::create_dir_all(&debian).map_err(|error| error.to_string())?;
    fs::write(debian.join("control"), "Source: sabine-package\n\nPackage: sabine-package\nArchitecture: any\nDescription: Application package\n").map_err(|error| error.to_string())?;
    let output = Command::new("dpkg-shlibdeps")
        .arg("-O")
        .arg(format!("-e{}", staged.binary.display()))
        .current_dir(&staged.root)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "could not calculate Debian dependencies: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout).map_err(|error| error.to_string())?;
    let libraries = output
        .lines()
        .find_map(|line| line.strip_prefix("shlibs:Depends="))
        .ok_or("dpkg-shlibdeps did not report shared library dependencies")?;
    Ok(if libraries.is_empty() {
        CEF_DEB_DEPENDENCIES.to_string()
    } else {
        format!("{libraries}, {CEF_DEB_DEPENDENCIES}")
    })
}

pub(super) fn rpm_spec(app: &BundleApp, executable: &str, binary: &Path) -> Result<String, String> {
    let license = app
        .license
        .as_deref()
        .ok_or("RPM packaging requires app.license or Cargo package license")?;
    let (_, architecture) = architecture(binary)?;
    Ok(format!(
        r#"Name: {name}
Version: {version}
Release: 1%{{?dist}}
Summary: {summary}
License: {license}
BuildArch: {architecture}
%global source_date_epoch_from_changelog 0
Requires: gtk3, nss, nspr, alsa-lib, cups-libs, libXcomposite, libXdamage, libXrandr, mesa-libgbm, libxkbcommon, libxkbcommon-x11, systemd-libs, wayland-libs

%description
{summary}

%prep

%build

%install
mkdir -p "%{{buildroot}}"
cp -a "%{{sabine_source}}/." "%{{buildroot}}/"

%files
/usr/bin/{executable}
/usr/share/applications/{id}.desktop
/usr/lib/sabine/{id}
"#,
        name = app.id.replace('.', "-"),
        version = package_version(&app.version)?.replace('-', "_"),
        summary = app.name.replace('%', "%%"),
        license = license.replace('%', "%%"),
        id = app.id
    ))
}

fn package_version(version: &str) -> Result<String, String> {
    let version = semver::Version::parse(version).map_err(|error| error.to_string())?;
    let core = format!("{}.{}.{}", version.major, version.minor, version.patch);
    Ok(if version.pre.is_empty() {
        core
    } else {
        format!("{core}~{}", version.pre)
    })
}
