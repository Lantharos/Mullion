use super::config::BundleApp;
use std::{fs, path::Path};

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
{icon}
!define MUI_ABORTWARNING
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

Section "Install"
  SetShellVarContext current
  ClearErrors
  SetOutPath "$INSTDIR"
  File /r "{source}"
  WriteUninstaller "$INSTDIR\Uninstall.exe"
  IfErrors setup_failed
  WriteRegStr HKCU "Software\{id}" "InstallDir" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "DisplayName" "{name}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "DisplayVersion" "{version}"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "InstallLocation" "$INSTDIR"
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "NoModify" 1
  WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\{id}" "NoRepair" 1
setup_retry:
  DetailPrint "Preparing the shared Sabine runtime..."
  nsExec::ExecToLog '"$INSTDIR\{executable}" --sabine-install'
  Pop $0
  StrCmp $0 "0" setup_ready
  IfSilent setup_failed
  MessageBox MB_RETRYCANCEL|MB_ICONEXCLAMATION "Setup could not finish. The details above explain the failure. Check your connection and retry, or cancel and rerun setup later." IDRETRY setup_retry
setup_failed:
  SetErrorLevel 1
  Abort
setup_ready:
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
