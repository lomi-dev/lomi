$ErrorActionPreference = 'Stop'
New-Item -ItemType Directory -Force installer-results | Out-Null
$root = $PWD.Path
$scratch = Join-Path $env:RUNNER_TEMP 'lomi-installer-test'
New-Item -ItemType Directory -Force $scratch | Out-Null

function Run-Installer($file, $arguments) {
    $process = Start-Process -FilePath $file -ArgumentList $arguments -PassThru
    if (-not $process.WaitForExit(180000)) { $process.Kill(); throw 'Installer timed out' }
    if ($process.ExitCode -notin @(0, 3010)) { throw "Installer failed: $($process.ExitCode)" }
}

foreach ($format in @('nsis', 'msi')) {
    $extension = if ($format -eq 'nsis') { 'exe' } else { 'msi' }
    $packages = @(Get-ChildItem "src-tauri/target/release/bundle/$format/*.$extension")
    if ($packages.Count -ne 1) { throw "Expected exactly one $format installer" }
    $package = $packages[0].FullName
    # Tauri remembers the previous install directory across uninstallers.
    # Exercise both formats in that same location, including NSIS -> MSI.
    $destination = Join-Path $scratch 'installed'
    $log = Join-Path $root "installer-results/$format-install.log"
    if ($format -eq 'nsis') {
        Run-Installer $package "/S /D=$destination"
    } else {
        Run-Installer msiexec.exe "/i `"$package`" /qn /norestart INSTALLDIR=`"$destination`" /l*v `"$log`""
    }
    $exe = Join-Path $destination 'lomi.exe'
    if (-not (Test-Path $exe)) { throw "Missing installed application at $exe" }
    node scripts/qualify-installers-runtime.mjs $exe $destination $format
    if ($LASTEXITCODE -ne 0) { throw "$format runtime qualification failed" }
    $app = Start-Process -FilePath $exe -PassThru
    try {
        $deadline = (Get-Date).AddSeconds(90)
        do {
            Start-Sleep -Seconds 1
            $app.Refresh()
            if ($app.HasExited) { throw "Installed $format app exited before showing a window" }
        } until ($app.MainWindowHandle -ne 0 -or (Get-Date) -ge $deadline)
        if ($app.MainWindowHandle -eq 0) { throw "Installed $format app did not show its window" }
        $app.MainWindowTitle | Set-Content "installer-results/$format-window.txt"
    } finally {
        if (-not $app.HasExited) {
            $null = $app.CloseMainWindow()
            if (-not $app.WaitForExit(10000)) { $app.Kill(); $app.WaitForExit() }
        }
    }
    if ($format -eq 'nsis') {
        Run-Installer (Join-Path $destination 'uninstall.exe') "/S _?=$destination"
    } else {
        $log = Join-Path $root 'installer-results/msi-uninstall.log'
        Run-Installer msiexec.exe "/x `"$package`" /qn /norestart /l*v `"$log`""
    }
    if (Test-Path $exe) { throw "$format uninstall left the application installed" }
    "$format installation, packaged runtime, visible GUI and uninstall passed." | Add-Content installer-results/windows.txt
}
