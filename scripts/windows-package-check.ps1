function Wait-InstallerExit([Diagnostics.Process] $Process) {
    if ($Process.WaitForExit(300000)) { return }
    Get-CimInstance Win32_Process |
        Where-Object { $_.Name -match 'sabine|release-probe|setup|msiexec' } |
        Select-Object ProcessId, ParentProcessId, Name, CommandLine | Format-List
    Get-ChildItem (Join-Path $env:LOCALAPPDATA 'sabine/logs') -Filter *.jsonl -ErrorAction SilentlyContinue |
        ForEach-Object { Get-Content $_.FullName -Tail 30 }
    Get-ChildItem $env:RUNNER_TEMP -Filter 'sabine-msi-*.log' -ErrorAction SilentlyContinue |
        ForEach-Object { Get-Content $_.FullName -Tail 100 }
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

function Test-MsiCancellation([string] $Package, [string] $Log) {
    Add-Type @'
using System;
using System.Diagnostics;
using System.Runtime.InteropServices;
public static class MsiCancellation {
    [UnmanagedFunctionPointer(CallingConvention.Winapi, CharSet = CharSet.Unicode)]
    private delegate int Handler(IntPtr context, uint kind, string message);
    [DllImport("msi.dll", CharSet = CharSet.Unicode)]
    private static extern uint MsiInstallProductW(string package, string properties);
    [DllImport("msi.dll", CharSet = CharSet.Unicode)]
    private static extern IntPtr MsiSetExternalUIW(IntPtr handler, uint filter, IntPtr context);
    [DllImport("msi.dll")]
    private static extern uint MsiSetInternalUI(uint level, IntPtr window);
    [DllImport("msi.dll", CharSet = CharSet.Unicode)]
    private static extern uint MsiEnableLogW(uint mode, string path, uint attributes);
    public static uint Install(string package, string log) {
        bool cancelled = false;
        bool sawSetup = false;
        var elapsed = Stopwatch.StartNew();
        Handler handler = (context, kind, message) => {
            if (message?.Contains("Sabine:") == true) {
                sawSetup = true;
                Console.WriteLine(message);
            }
            uint category = kind & 0xff000000;
            if (!cancelled && (sawSetup || elapsed.Elapsed.TotalSeconds > 180) &&
                (category == 0x09000000 || category == 0x0a000000)) {
                cancelled = true;
                return 2;
            }
            return 0;
        };
        uint previousLevel = MsiSetInternalUI(2, IntPtr.Zero);
        IntPtr previousHandler = MsiSetExternalUIW(Marshal.GetFunctionPointerForDelegate(handler), 0x1fff, IntPtr.Zero);
        try {
            uint logging = MsiEnableLogW(0x1fff, log, 2);
            if (logging != 0) throw new Exception("MSI logging failed: " + logging);
            uint result = MsiInstallProductW(package, "REBOOT=ReallySuppress");
            if (!sawSetup) throw new Exception("MSI did not expose cancellable setup progress");
            return result;
        } finally {
            MsiEnableLogW(0, null, 0);
            MsiSetExternalUIW(previousHandler, 0, IntPtr.Zero);
            MsiSetInternalUI(previousLevel, IntPtr.Zero);
            GC.KeepAlive(handler);
        }
    }
}
'@
    $result = [MsiCancellation]::Install($Package, $Log)
    if ($result -ne 1602) {
        Select-String -Path $Log -Pattern 'Sabine:|CustomAction|Return value 3|error|cancel' -Context 2,2
        throw "Cancelled MSI returned $result instead of 1602"
    }
}
