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
        Write-Host 'Installed Chromium rendered and verified its probe page'
    } finally {
        if (-not $browser.HasExited) { $browser.Kill($true); $browser.WaitForExit() }
        $browser.Dispose()
        if (Test-Path $profile) { Remove-Item $profile -Recurse -Force }
    }
}
