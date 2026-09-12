use super::config::BundleApp;
use std::{fs, path::Path};

pub(super) fn wix_source(
    app: &BundleApp,
    staged_app_dir: &str,
    executable: &str,
    icon: Option<&str>,
) -> Result<String, String> {
    let version = semver::Version::parse(&app.version).map_err(|error| error.to_string())?;
    if version.major > 255 || version.minor > 255 || version.patch > 65535 {
        return Err("MSI versions require major/minor <= 255 and patch <= 65535".into());
    }
    let version = format!("{}.{}.{}", version.major, version.minor, version.patch);
    let upgrade_code = deterministic_guid(&app.id);
    let icon_element = icon
        .map(|icon| {
            format!(
                "    <Icon Id=\"AppIcon.ico\" SourceFile=\"{}\"/>\n",
                xml(icon)
            )
        })
        .unwrap_or_default();
    let inventory = directory_inventory(
        app,
        Path::new(staged_app_dir),
        Path::new(staged_app_dir),
        executable,
        icon.is_some(),
    )?;
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<Wix xmlns="http://wixtoolset.org/schemas/v4/wxs" xmlns:ui="http://wixtoolset.org/schemas/v4/wxs/ui">
    <Package Name="{}" Manufacturer="{}" Version="{}" UpgradeCode="{}" Scope="perUser">
      <MediaTemplate EmbedCab="yes"/>
      <MajorUpgrade Schedule="afterInstallInitialize" DowngradeErrorMessage="A newer version of this application is already installed."/>
    <StandardDirectory Id="LocalAppDataFolder">
      <Directory Id="ProgramsFolder" Name="Programs">
        <Directory Id="INSTALLFOLDER" Name="{}">
{}
        </Directory>
      </Directory>
    </StandardDirectory>
    <StandardDirectory Id="ProgramMenuFolder"/>
{}
    <ui:WixUI Id="WixUI_InstallDir" InstallDirectory="INSTALLFOLDER"/>
    <UI>
      <Publish Dialog="WelcomeDlg" Control="Next" Event="NewDialog" Value="InstallDirDlg" Order="2" Condition="NOT Installed"/>
      <Publish Dialog="InstallDirDlg" Control="Back" Event="NewDialog" Value="WelcomeDlg" Order="2"/>
      <ProgressText Action="SabinePrepare" Message="Preparing the shared Sabine runtime"/>
      <ProgressText Action="SabineUnregister" Message="Removing application registration"/>
    </UI>
    <UIRef Id="WixUI_ErrorProgressText"/>
{}
    <InstallExecuteSequence>
      <Custom Action="SabineRollbackPrepare" Before="SabinePrepare" Condition="NOT Installed"/>
      <Custom Action="SabinePrepare" After="InstallFiles" Condition="NOT REMOVE~=&quot;ALL&quot;"/>
      <Custom Action="SabineRollbackUnregister" Before="SabineUnregister" Condition="REMOVE~=&quot;ALL&quot;"/>
      <Custom Action="SabineUnregister" Before="RemoveFiles" Condition="REMOVE~=&quot;ALL&quot;"/>
    </InstallExecuteSequence>
  </Package>
</Wix>
"#,
        xml(&app.name),
        xml(&app.publisher),
        xml(&version),
        upgrade_code,
        xml(&app.id),
        inventory,
        icon_element,
        actions(),
    ))
}

fn directory_inventory(
    app: &BundleApp,
    root: &Path,
    directory: &Path,
    executable: &str,
    has_icon: bool,
) -> Result<String, String> {
    let relative = directory
        .strip_prefix(root)
        .map_err(|error| error.to_string())?;
    let identity = format!(
        "{}:{}",
        app.id,
        relative.to_string_lossy().replace('\\', "/")
    );
    let guid = deterministic_guid(&identity);
    let id = format!("D{}", guid.replace('-', ""));
    let mut entries = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    let registry = xml(&format!("Software\\{}\\InstallerComponents", app.id));
    let mut contents = format!(
        r#"<Component Id="C{id}" Guid="{guid}">
<RegistryValue Root="HKCU" Key="{registry}" Name="{id}" Type="integer" Value="1" KeyPath="yes"/>
<CreateFolder/>
<RemoveFolder Id="R{id}" On="uninstall"/>
"#
    );
    let mut children = String::new();
    for entry in entries {
        let path = entry.path();
        let kind = entry.file_type().map_err(|error| error.to_string())?;
        if kind.is_dir() {
            children.push_str(&directory_inventory(
                app, root, &path, executable, has_icon,
            )?);
        } else if kind.is_file() {
            let main = directory == root && entry.file_name() == executable;
            let file_id = if main {
                r#" Id="MainExecutableFile""#
            } else {
                ""
            };
            contents.push_str(&format!(
                r#"<File{file_id} Source="{}"/>"#,
                xml(&path.display().to_string())
            ));
            if main {
                let icon = if has_icon {
                    r#" Icon="AppIcon.ico""#
                } else {
                    ""
                };
                contents.push_str(&format!(r#"<Shortcut Id="StartMenuShortcut" Directory="ProgramMenuFolder" Name="{}" Target="[#MainExecutableFile]" WorkingDirectory="INSTALLFOLDER"{icon}/>"#, xml(&app.name)));
            }
        } else {
            return Err(format!(
                "MSI payload must contain regular files and directories: {}",
                path.display()
            ));
        }
    }
    contents.push_str("</Component>\n");
    contents.push_str(&children);
    if relative.as_os_str().is_empty() {
        Ok(contents)
    } else {
        let name = xml(&directory.file_name().unwrap().to_string_lossy());
        Ok(format!(
            r#"<Directory Id="{id}" Name="{name}">{contents}</Directory>"#
        ))
    }
}

fn actions() -> String {
    [
        ("SabinePrepare", "--sabine-install", "deferred", "check"),
        ("SabineRollbackPrepare", "--sabine-uninstall", "rollback", "ignore"),
        ("SabineUnregister", "--sabine-uninstall", "deferred", "check"),
        ("SabineRollbackUnregister", "--sabine-install", "rollback", "ignore"),
    ]
    .into_iter()
    .map(|(id, argument, execute, result)| format!(
        r#"    <SetProperty Id="{id}" Value="&quot;[#MainExecutableFile]&quot; {argument}" Before="{id}" Sequence="execute"/>
    <CustomAction Id="{id}" BinaryRef="Wix4UtilCA_$(sys.BUILDARCHSHORT)" DllEntry="WixQuietExec" Execute="{execute}" Impersonate="yes" Return="{result}"/>
"#
    ))
    .collect()
}

fn deterministic_guid(value: &str) -> String {
    use sha2::{Digest, Sha256};
    let mut bytes: [u8; 16] = Sha256::digest(value.as_bytes())[..16]
        .try_into()
        .expect("SHA-256 prefix has a fixed length");
    bytes[6] = (bytes[6] & 0x0f) | 0x50;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    format!(
        "{:02X}{:02X}{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
        bytes[0],
        bytes[1],
        bytes[2],
        bytes[3],
        bytes[4],
        bytes[5],
        bytes[6],
        bytes[7],
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}
