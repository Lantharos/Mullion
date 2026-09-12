function Wait-InstallerExit([Diagnostics.Process] $Process) {
    if ($Process.WaitForExit(300000)) { return }
    Get-CimInstance Win32_Process |
        Where-Object { $_.Name -match 'sabine|release-probe|setup|msiexec' } |
        Select-Object ProcessId, ParentProcessId, Name, CommandLine | Format-List
    Get-ChildItem (Join-Path $env:LOCALAPPDATA 'sabine/logs') -Filter *.jsonl -ErrorAction SilentlyContinue |
        ForEach-Object { Get-Content $_.FullName -Tail 30 }
    Stop-Process -Id $Process.Id -Force -ErrorAction SilentlyContinue
    throw 'Installer did not exit within five minutes'
}
