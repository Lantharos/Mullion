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

function Test-SetupCancellation([string] $Setup, [string] $Destination) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstallerWindow {
    [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr window, int id);
    [DllImport("user32.dll")] public static extern bool IsWindowEnabled(IntPtr window);
    [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wparam, IntPtr lparam);
}
'@
    $process = Start-Process $Setup -ArgumentList "/D=$Destination" -PassThru
    try {
        $deadline = [DateTime]::UtcNow.AddSeconds(120)
        do {
            $process.Refresh()
            $window = $process.MainWindowHandle
            if ($process.HasExited -or [DateTime]::UtcNow -gt $deadline) { throw 'Setup did not open its wizard' }
            Start-Sleep -Milliseconds 20
        } while ($window -eq 0)
        foreach ($page in @('Welcome', 'Directory')) {
            $next = [InstallerWindow]::GetDlgItem($window, 1)
            while (-not [InstallerWindow]::IsWindowEnabled($next)) {
                if ([DateTime]::UtcNow -gt $deadline) { throw "Setup did not reach $page" }
                Start-Sleep -Milliseconds 20
            }
            [void][InstallerWindow]::SendMessage($window, 0x111, [IntPtr]1, $next)
            Start-Sleep -Milliseconds 100
        }
        $cancel = [InstallerWindow]::GetDlgItem($window, 2)
        while (-not [InstallerWindow]::IsWindowEnabled($cancel)) {
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Setup did not enable cancellation during preparation' }
            Start-Sleep -Milliseconds 10
        }
        [void][InstallerWindow]::SendMessage($window, 0x111, [IntPtr]2, $cancel)
        while (-not [InstallerWindow]::IsWindowEnabled($cancel)) {
            if ([DateTime]::UtcNow -gt $deadline) { throw 'Setup did not finish cancelling' }
            Start-Sleep -Milliseconds 20
        }
        [void][InstallerWindow]::SendMessage($window, 0x111, [IntPtr]2, $cancel)
        Wait-InstallerExit $process
        if ($process.ExitCode -ne 1602) { throw "Cancelled setup returned $($process.ExitCode), expected 1602" }
        if (Test-Path $Destination) { throw 'Cancelled setup published an application directory' }
    } finally {
        if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    }
}
