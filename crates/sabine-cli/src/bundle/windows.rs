use super::config::BundleApp;
use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::Path,
};

pub(super) fn nsis_script(
    app: &BundleApp,
    source: &Path,
    executable: &str,
    output: &str,
    icon: Option<&str>,
) -> Result<String, String> {
    let source = dunce::canonicalize(source).map_err(|error| error.to_string())?;
    let name = escape(&app.name);
    let id = escape(&app.id);
    let executable = escape(executable);
    let uninstall_files = uninstall_commands(&source, &source)?;
    let mut files = payload_files(&source, &source)?;
    files.push("Uninstall.exe".into());
    fs::write(
        source.join(".sabine-install.json"),
        serde_json::to_vec(&serde_json::json!({"id": app.id, "files": files}))
            .map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let icon = icon
        .map(|path| {
            format!(
                "Icon \"{}\"\nUninstallIcon \"{}\"",
                escape(&dunce::simplified(Path::new(path)).to_string_lossy()),
                escape(&dunce::simplified(Path::new(path)).to_string_lossy())
            )
        })
        .unwrap_or_default();
    Ok(format!(
        r#"Unicode true
!include "MUI2.nsh"
Name "{name}"
OutFile "{output}"
RequestExecutionLevel user
InstallDir "$LOCALAPPDATA\Programs\{id}"
InstallDirRegKey HKCU "Software\{id}" "InstallDir"
SetCompressor /SOLID lzma
ShowInstDetails show
ShowUninstDetails show
Var SetupRunning
Var SetupCancelled
Var CancelHandle
Var CancelButton
{icon}
!define MUI_CUSTOMFUNCTION_ABORT CancelSetup
!define MUI_WELCOMEPAGE_TEXT "Setup installs {name} and prepares its shared Sabine runtime.$\r$\n$\r$\nIf the runtime is not already available, setup downloads it before finishing. You can rerun this installer to repair the installation."
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\{executable}"
!define MUI_FINISHPAGE_RUN_NOTCHECKED
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"

Function CancelSetup
  StrCmp $SetupRunning "1" 0 cancel_normal
  StrCpy $SetupCancelled "1"
  FileOpen $CancelHandle "$PLUGINSDIR\cancel" w
  FileClose $CancelHandle
  GetDlgItem $CancelButton $HWNDPARENT 2
  EnableWindow $CancelButton 0
  DetailPrint "Cancelling setup..."
  Abort
cancel_normal:
FunctionEnd

Section "Install"
  SetShellVarContext current
  InitPluginsDir
  ClearErrors
  SetOutPath "$PLUGINSDIR\payload"
  File /r "{source}"
  WriteUninstaller "$PLUGINSDIR\payload\Uninstall.exe"
  IfErrors setup_failed
setup_retry:
  Delete "$PLUGINSDIR\cancel"
  StrCpy $SetupCancelled "0"
  StrCpy $SetupRunning "1"
  GetDlgItem $CancelButton $HWNDPARENT 2
  EnableWindow $CancelButton 1
  DetailPrint "Preparing the shared Sabine runtime..."
  nsExec::ExecToLog '"$PLUGINSDIR\payload\{executable}" --sabine-install --sabine-install-to "$INSTDIR" --sabine-install-cancel "$PLUGINSDIR\cancel"'
  Pop $0
  StrCpy $SetupRunning "0"
  GetDlgItem $CancelButton $HWNDPARENT 2
  EnableWindow $CancelButton 0
  StrCmp $0 "0" setup_ready
  StrCmp $SetupCancelled "1" setup_cancelled
  IfSilent setup_failed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Setup could not finish. The details above explain the failure. Close the application if it is running, check your connection and retry." IDRETRY setup_retry
setup_failed:
  SetErrorLevel 1
  Abort
setup_cancelled:
  SetErrorLevel 1602
  Abort "Installation cancelled."
setup_ready:
  WriteRegStr HKCU "Software\{id}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "DisplayName" "{name}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "DisplayVersion" "{version}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "NoRepair" 1
  CreateDirectory "$SMPROGRAMS\{name}"
  CreateShortcut "$SMPROGRAMS\{name}\{name}.lnk" "$INSTDIR\{executable}"
  CreateShortcut "$SMPROGRAMS\{name}\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
SectionEnd

Section "Uninstall"
  SetShellVarContext current
  IfFileExists "$INSTDIR\{executable}" 0 unregister_done
unregister_retry:
  nsExec::ExecToLog '"$INSTDIR\{executable}" --sabine-uninstall'
  Pop $0
  StrCmp $0 "0" unregister_done
  IfSilent unregister_failed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "The application could not be unregistered. Close the application and retry." IDRETRY unregister_retry
unregister_failed:
  SetErrorLevel 1
  Abort
unregister_done:
{uninstall_files}
  Delete "$INSTDIR\.sabine-install.json"
  Delete "$SMPROGRAMS\{name}\{name}.lnk"
  Delete "$SMPROGRAMS\{name}\Uninstall.lnk"
  RMDir "$SMPROGRAMS\{name}"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir "$INSTDIR"
  DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}"
  DeleteRegKey HKCU "Software\{id}"
SectionEnd
"#,
        output = escape(&dunce::simplified(Path::new(output)).to_string_lossy()),
        source = escape(&source.join("*").display().to_string()),
        version = escape(&app.version),
    ))
}

fn payload_files(root: &Path, directory: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            files.extend(payload_files(root, &path)?);
        } else {
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_string_lossy()
                .replace('\\', "/");
            if relative != ".sabine-install.json" && relative != "Uninstall.exe" {
                files.push(relative);
            }
        }
    }
    files.sort();
    Ok(files)
}

fn uninstall_commands(root: &Path, directory: &Path) -> Result<String, String> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut commands = String::new();
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).map_err(|error| error.to_string())?;
        if relative == Path::new(".sabine-install.json") || relative == Path::new("Uninstall.exe") {
            continue;
        }
        let target = escape(&relative.to_string_lossy().replace('/', "\\"));
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            commands.push_str(&uninstall_commands(root, &path)?);
            commands.push_str(&format!("  RMDir \"$INSTDIR\\{target}\"\n"));
        } else {
            commands.push_str(&format!(
                "  ClearErrors\n  Delete \"$INSTDIR\\{target}\"\n  IfErrors unregister_failed\n"
            ));
        }
    }
    Ok(commands)
}

fn escape(value: &str) -> String {
    value
        .replace('$', "$$")
        .replace('"', "$\\\"")
        .replace('\r', "$\\r")
        .replace('\n', "$\\n")
}

pub(super) fn architecture(binary: &Path) -> Result<&'static str, String> {
    let mut file = fs::File::open(binary).map_err(|error| error.to_string())?;
    let mut dos = [0_u8; 64];
    file.read_exact(&mut dos)
        .map_err(|error| error.to_string())?;
    if &dos[..2] != b"MZ" {
        return Err("Windows packages require a PE executable".into());
    }
    let offset = u32::from_le_bytes(dos[60..64].try_into().unwrap());
    file.seek(SeekFrom::Start(u64::from(offset)))
        .map_err(|error| error.to_string())?;
    let mut pe = [0_u8; 26];
    file.read_exact(&mut pe)
        .map_err(|error| error.to_string())?;
    if &pe[..4] != b"PE\0\0" {
        return Err("Windows executable has an invalid PE signature".into());
    }
    let characteristics = u16::from_le_bytes([pe[22], pe[23]]);
    if characteristics & 0x2002 != 0x0002 || pe[24..26] != [0x0b, 0x02] {
        return Err("Windows packages require a 64-bit PE application executable".into());
    }
    match u16::from_le_bytes([pe[4], pe[5]]) {
        0x8664 => Ok("x64"),
        0xaa64 => Ok("ARM64"),
        _ => Err("Windows packages require an x86_64 or ARM64 executable".into()),
    }
}
