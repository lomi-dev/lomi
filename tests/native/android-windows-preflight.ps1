param([Parameter(Mandatory = $true)][string]$OutputDirectory)

$ErrorActionPreference = 'Stop'
$repo = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path
$output = [IO.Path]::GetFullPath($OutputDirectory)
if (Test-Path -LiteralPath $output) { throw 'Use a new evidence directory.' }
New-Item -ItemType Directory -Path $output | Out-Null
$results = [ordered]@{ capturedAt = [DateTime]::UtcNow.ToString('o'); checks = [ordered]@{} }
function Record([string]$Name, [scriptblock]$Read) {
    try { $results.checks[$Name] = @{ status = 'PASS'; value = (& $Read) } }
    catch { $results.checks[$Name] = @{ status = 'NOT RUN'; error = $_.Exception.Message } }
}

Record 'os' { Get-CimInstance Win32_OperatingSystem | Select-Object Caption, Version, BuildNumber, OSArchitecture, TotalVisibleMemorySize }
Record 'computer' { Get-CimInstance Win32_ComputerSystem | Select-Object Manufacturer, Model, HypervisorPresent, TotalPhysicalMemory }
Record 'cpu' { Get-CimInstance Win32_Processor | Select-Object Name, NumberOfCores, NumberOfLogicalProcessors, VirtualizationFirmwareEnabled, SecondLevelAddressTranslationExtensions }
Record 'gpu' { Get-CimInstance Win32_VideoController | Select-Object Name, DriverVersion, CurrentHorizontalResolution, CurrentVerticalResolution }
Record 'disk' { Get-CimInstance Win32_LogicalDisk -Filter 'DriveType=3' | Select-Object DeviceID, Size, FreeSpace }
Record 'optionalFeature' { Get-WindowsOptionalFeature -Online -FeatureName HypervisorPlatform | Select-Object FeatureName, State }
Record 'whpx' {
    Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class AndroidPreflight {
    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)]
    static extern IntPtr LoadLibraryExW(string name, IntPtr file, uint flags);
    [DllImport("kernel32", CharSet = CharSet.Ansi)]
    static extern IntPtr GetProcAddress(IntPtr library, string name);
    [DllImport("kernel32")]
    static extern bool FreeLibrary(IntPtr library);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    delegate int Query(int code, out int present, uint size, out uint written);
    public static bool Present() {
        var library = LoadLibraryExW("WinHvPlatform.dll", IntPtr.Zero, 0x800);
        if (library == IntPtr.Zero) throw new System.ComponentModel.Win32Exception(Marshal.GetLastWin32Error());
        try {
            var address = GetProcAddress(library, "WHvGetCapability");
            if (address == IntPtr.Zero) throw new Exception("WHvGetCapability is absent");
            var query = (Query)Marshal.GetDelegateForFunctionPointer(address, typeof(Query));
            int present; uint written;
            int result = query(0, out present, 4, out written);
            Marshal.ThrowExceptionForHR(result);
            if (written != 4) throw new Exception("Unexpected WHPX capability size");
            return present != 0;
        } finally { FreeLibrary(library); }
    }
    [DllImport("user32")]
    public static extern uint GetDpiForSystem();
}
'@
    @{ available = [AndroidPreflight]::Present() }
}
if ($results.checks.whpx.status -eq 'PASS' -and !$results.checks.whpx.value.available) {
    $results.checks.whpx.status = 'FAIL'
}
Record 'systemDpi' { [AndroidPreflight]::GetDpiForSystem() }
Record 'webview2' {
    $versions = @()
    foreach ($base in @("${env:ProgramFiles(x86)}/Microsoft/EdgeWebView/Application", "$env:LOCALAPPDATA/Microsoft/EdgeWebView/Application")) {
        if (Test-Path -LiteralPath $base) {
            $versions += Get-ChildItem -LiteralPath $base -Directory | ForEach-Object {
                $binary = Join-Path $_.FullName 'msedgewebview2.exe'
                if (Test-Path -LiteralPath $binary) { (Get-Item -LiteralPath $binary).VersionInfo.FileVersion }
            }
        }
    }
    if (!$versions.Count) { throw 'WebView2 runtime not found in standard locations.' }
    $versions
}
Record 'inheritedConfiguration' {
    # Record names only: environment values and user configuration may contain secrets.
    @{
        variableNames = @(Get-ChildItem Env: | Where-Object Name -Match '^(ANDROID_|ADB_|JAVA_|JDK_|_JAVA_|REPO_|QT_|SDKMANAGER_|AVDMANAGER_)|^(CLASSPATH|LD_PRELOAD|LD_LIBRARY_PATH)$' | Select-Object -ExpandProperty Name)
        androidrcPresent = Test-Path -LiteralPath (Join-Path $env:USERPROFILE '.androidrc')
    }
}
Record 'integrity' {
    $manifest = Get-Content -Raw -LiteralPath (Join-Path $repo 'src-tauri/android-input/artifact.json') | ConvertFrom-Json
    $files = @(@{ path = 'src-tauri/android-input/simplebench-input.apk'; expected = $manifest.apkSha256 })
    foreach ($property in $manifest.sources.PSObject.Properties) {
        $files += @{ path = "src-tauri/android-input/$($property.Name)"; expected = $property.Value }
    }
    $files += @{ path = 'src-tauri/android-proto/emulator_controller.proto'; expected = '1d62c6bcad5f06621f90ec2bf26c661ba769ccd0f1416b5314d25a68e04eee5f' }
    foreach ($file in $files) {
        $hash = (Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $repo $file.path)).Hash.ToLowerInvariant()
        [pscustomobject]@{ path = $file.path; sha256 = $hash; matches = ($hash -eq $file.expected) }
    }
}
if ($results.checks.integrity.status -eq 'PASS' -and @($results.checks.integrity.value | Where-Object { !$_.matches }).Count) {
    $results.checks.integrity.status = 'FAIL'
}
Record 'qualificationGate' {
    (Get-Content -Raw -LiteralPath (Join-Path $repo 'src-tauri/android-toolchain.json') | ConvertFrom-Json).hosts.windows_x86_64.qualified
}
$results | ConvertTo-Json -Depth 10 | Set-Content -Encoding UTF8 -LiteralPath (Join-Path $output 'preflight.json')
Write-Output (Join-Path $output 'preflight.json')
if ($results.checks.whpx.status -ne 'PASS' -or $results.checks.integrity.status -ne 'PASS') { exit 2 }
