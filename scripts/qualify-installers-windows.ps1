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

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class LomiInstallerWindow {
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr window, int command);
    [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr window);
    [DllImport("user32.dll")] public static extern bool IsIconic(IntPtr window);
}
'@

function Wait-AppWindow($process, $format) {
    $deadline = (Get-Date).AddSeconds(90)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "Installed $format app exited before showing a window" }
        if ($process.MainWindowHandle -ne 0) { return $process.MainWindowHandle }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "Installed $format app did not show its window"
}

function Assert-DuplicateExit($process, $format) {
    if (-not $process.WaitForExit(15000)) { throw "$format duplicate launch did not exit within 15 seconds" }
    if ($process.ExitCode -ne 0) { throw "$format duplicate launch failed: $($process.ExitCode)" }
}

function Assert-Restored($process, $window, $format) {
    $deadline = (Get-Date).AddSeconds(15)
    do {
        $process.Refresh()
        if ($process.HasExited) { throw "$format duplicate launch terminated the primary process" }
        if ([LomiInstallerWindow]::IsWindowVisible($window) -and -not [LomiInstallerWindow]::IsIconic($window)) { return }
        Start-Sleep -Milliseconds 200
    } while ((Get-Date) -lt $deadline)
    throw "$format duplicate launch did not restore the primary window"
}

function Close-App($process, $format) {
    # Let asynchronous frontend initialization attach its close/save guards.
    Start-Sleep -Seconds 2
    if (-not $process.CloseMainWindow()) { throw "$format app did not accept its window close request" }
    if (-not $process.WaitForExit(30000)) { throw "$format window close left process $($process.Id) running after 30 seconds" }
    if ($process.ExitCode -ne 0) { throw "$format app exited with code $($process.ExitCode) after window close" }
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
    $processes = [System.Collections.Generic.List[System.Diagnostics.Process]]::new()
    try {
        # Either launch can acquire the instance guard first during startup.
        $first = Start-Process -FilePath $exe -PassThru
        $processes.Add($first)
        $second = Start-Process -FilePath $exe -PassThru
        $processes.Add($second)
        $deadline = (Get-Date).AddSeconds(15)
        do {
            $first.Refresh()
            $second.Refresh()
            if ($first.HasExited -and $second.HasExited) { throw "Installed $format startup launches both exited" }
            if ($first.HasExited -or $second.HasExited) { break }
            Start-Sleep -Milliseconds 200
        } while ((Get-Date) -lt $deadline)
        if (-not $first.HasExited -and -not $second.HasExited) { throw "$format startup burst left two processes running after 15 seconds" }
        $app = if ($first.HasExited) { $second } else { $first }
        $duplicate = if ($app.Id -eq $first.Id) { $second } else { $first }
        Assert-DuplicateExit $duplicate $format
        $window = Wait-AppWindow $app $format
        Assert-Restored $app $window $format
        $app.MainWindowTitle | Set-Content "installer-results/$format-window.txt"
        "$format startup burst: primary PID $($app.Id), duplicate PID $($duplicate.Id) exited successfully." | Add-Content installer-results/windows.txt

        $null = [LomiInstallerWindow]::ShowWindow($window, 6)
        if (-not [LomiInstallerWindow]::IsIconic($window)) { throw "$format primary window did not minimize" }
        $duplicate = Start-Process -FilePath $exe -PassThru
        $processes.Add($duplicate)
        Assert-DuplicateExit $duplicate $format
        Assert-Restored $app $window $format
        "$format second launch exited successfully and restored minimized primary PID $($app.Id)." | Add-Content installer-results/windows.txt

        # An executable at another installation path must use the same guard.
        $copyDirectory = Join-Path $scratch "$format-duplicate"
        New-Item -ItemType Directory -Force $copyDirectory | Out-Null
        $copy = Join-Path $copyDirectory 'lomi.exe'
        Copy-Item $exe $copy -Force
        $null = [LomiInstallerWindow]::ShowWindow($window, 0)
        if ([LomiInstallerWindow]::IsWindowVisible($window)) { throw "$format primary window did not hide" }
        $duplicate = Start-Process -FilePath $copy -PassThru
        $processes.Add($duplicate)
        Assert-DuplicateExit $duplicate $format
        Assert-Restored $app $window $format
        "$format copied executable launch exited successfully and restored hidden primary PID $($app.Id) from a different executable path." | Add-Content installer-results/windows.txt

        Close-App $app $format
        $previousId = $app.Id
        "$format window close exited primary PID $previousId successfully within 30 seconds." | Add-Content installer-results/windows.txt
        $app = Start-Process -FilePath $exe -PassThru
        $processes.Add($app)
        $window = Wait-AppWindow $app $format
        Assert-Restored $app $window $format
        Close-App $app $format
        "$format reopened a fresh process (PID $($app.Id), previous PID $previousId), displayed its window, and exited successfully on close within 30 seconds." | Add-Content installer-results/windows.txt
    } finally {
        foreach ($process in $processes) {
            $process.Refresh()
            if (-not $process.HasExited) {
                Write-Warning "Failure cleanup: killing remaining $format application PID $($process.Id)."
                $process.Kill()
                $process.WaitForExit()
            }
        }
    }
    if ($format -eq 'nsis') {
        Run-Installer (Join-Path $destination 'uninstall.exe') "/S _?=$destination"
    } else {
        $log = Join-Path $root 'installer-results/msi-uninstall.log'
        Run-Installer msiexec.exe "/x `"$package`" /qn /norestart /l*v `"$log`""
    }
    if (Test-Path $exe) { throw "$format uninstall left the application installed" }
    "$format installation, packaged runtime, single instance startup/restoration, strict window-close exit, reopen and uninstall passed." | Add-Content installer-results/windows.txt
}
