function Assert-SabineBrowserDiagnostics([string] $Diagnostics) {
    if ($Diagnostics -match 'Invalid file descriptor to ICU|Failed to load [^\r\n]*\.pak|socket read failed|recovering surface|The browser stopped repeatedly') {
        throw "Browser startup diagnostics report a failure: $Diagnostics"
    }
}

function Wait-SabineAppFrame([Diagnostics.Process] $Application, [string] $LogPath) {
    $deadline = (Get-Date).AddSeconds(60)
    $firstPaint = $null
    while ((Get-Date) -lt $deadline) {
        $diagnostics = ''
        if (Test-Path $LogPath) {
            $file = [IO.File]::Open($LogPath, [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::ReadWrite)
            $reader = [IO.StreamReader]::new($file)
            try { $diagnostics = $reader.ReadToEnd() } finally { $reader.Dispose() }
        }
        Assert-SabineBrowserDiagnostics $diagnostics
        if ($Application.HasExited) {
            throw "Installed app exited with code $($Application.ExitCode): $diagnostics"
        }
        if ($null -eq $firstPaint -and $diagnostics -match 'osr-host pid=\d+ browser\.first_paint') {
            $firstPaint = Get-Date
        }
        if ($null -ne $firstPaint -and ((Get-Date) - $firstPaint).TotalSeconds -ge 5) {
            Write-Host 'Installed application presented its Chromium frame and kept its OSR connection alive'
            return
        }
        Start-Sleep -Milliseconds 100
    }
    throw "Installed app did not present its Chromium frame: $diagnostics"
}

function Test-SabineBrowser {
    $sabineData = Join-Path $env:LOCALAPPDATA 'Sabine'
    $current = Get-Content (Join-Path $sabineData 'bin/current.json') -Raw | ConvertFrom-Json
    $hostPath = Join-Path $sabineData "bin/versions/$($current.active)/sabine-host.exe"
    $libraries = @(Get-ChildItem (Join-Path $sabineData 'runtimes/cef') -Filter libcef.dll -File -Recurse)
    if ($libraries.Count -ne 1) { throw 'Expected one installed Chromium runtime' }
    $binaryDirectory = $libraries[0].Directory.FullName
    $resources = Join-Path (Split-Path $binaryDirectory) 'Resources'
    $profile = Join-Path $env:RUNNER_TEMP "sabine-browser-check-$([guid]::NewGuid())"
    $start = [Diagnostics.ProcessStartInfo]::new($hostPath)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.WorkingDirectory = $binaryDirectory
    $start.Environment['PATH'] = "$binaryDirectory;$env:PATH"
    foreach ($argument in @(
        '--sabine-runtime-smoke-test',
        "--sabine-resources-dir-path=$resources",
        "--sabine-locales-dir-path=$(Join-Path $resources 'locales')",
        "--root-cache-path=$profile"
    )) { $start.ArgumentList.Add($argument) }
    $browser = [Diagnostics.Process]::Start($start)
    $output = $browser.StandardOutput.ReadToEndAsync()
    $errors = $browser.StandardError.ReadToEndAsync()
    try {
        if (-not $browser.WaitForExit(35000)) {
            $browser.Kill($true)
            $browser.WaitForExit()
            throw 'Chromium did not finish its rendering check'
        }
        if ($browser.ExitCode -ne 0) {
            throw "Chromium rendering failed ($($browser.ExitCode)): $($output.Result) $($errors.Result)"
        }
        Assert-SabineBrowserDiagnostics $errors.Result
        Write-Host 'Installed Chromium rendered and verified its probe page'
    } finally {
        if (-not $browser.HasExited) { $browser.Kill($true); $browser.WaitForExit() }
        $browser.Dispose()
        if (Test-Path $profile) { Remove-Item $profile -Recurse -Force }
    }
}
