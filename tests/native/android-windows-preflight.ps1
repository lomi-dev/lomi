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
    [DllImport("kernel32", CharSet = CharSet.Ansi, SetLastError = true)]
    static extern IntPtr GetProcAddress(IntPtr library, string name);
    [DllImport("kernel32")]
    static extern bool FreeLibrary(IntPtr library);
    [UnmanagedFunctionPointer(CallingConvention.Winapi)]
    delegate int Query(int code, out long capability, uint size, out uint written);
    public sealed class CapabilityDiagnostic {
        public bool available;
        public string diagnostic;
        public bool dllLoaded;
        public bool entryPointFound;
        public int? win32Error;
        public string hresult;
        public uint bufferSize = 8;
        public uint? written;
        public int? present;
    }
    public static CapabilityDiagnostic Probe() {
        var evidence = new CapabilityDiagnostic();
        var library = LoadLibraryExW("WinHvPlatform.dll", IntPtr.Zero, 0x800);
        if (library == IntPtr.Zero) {
            evidence.win32Error = Marshal.GetLastWin32Error();
            evidence.diagnostic = "WinHvPlatform.dll could not load from System32; Windows Hypervisor Platform may be missing or require repair: " + new System.ComponentModel.Win32Exception(evidence.win32Error.Value).Message;
            return evidence;
        }
        evidence.dllLoaded = true;
        try {
            var address = GetProcAddress(library, "WHvGetCapability");
            if (address == IntPtr.Zero) {
                evidence.win32Error = Marshal.GetLastWin32Error();
                evidence.diagnostic = "WinHvPlatform.dll loaded, but WHvGetCapability is absent.";
                return evidence;
            }
            evidence.entryPointFound = true;
            var query = (Query)Marshal.GetDelegateForFunctionPointer(address, typeof(Query));
            // The API requires at least 8 aligned bytes, even for the four-byte BOOL.
            long capability; uint written;
            int result = query(0, out capability, evidence.bufferSize, out written);
            evidence.hresult = "0x" + result.ToString("X8");
            evidence.written = written;
            if (result < 0) {
                evidence.diagnostic = "WHvGetCapability failed (HRESULT=" + evidence.hresult + ").";
            } else if (written != 4) {
                evidence.diagnostic = "WHvGetCapability returned an unexpected capability size (written=" + written + "; expected 4 bytes).";
            } else {
                evidence.present = unchecked((int)capability);
                evidence.available = evidence.present.Value != 0;
                evidence.diagnostic = evidence.available
                    ? "WHvGetCapability reports HypervisorPresent=true."
                    : "WHvGetCapability succeeded and reports HypervisorPresent=false; no running hypervisor is available. Check firmware virtualization, Windows Hypervisor Platform and nested virtualization when running in a host VM, then restart Windows.";
            }
            return evidence;
        } finally { FreeLibrary(library); }
    }
    [DllImport("user32")]
    public static extern uint GetDpiForSystem();
    [DllImport("kernel32")]
    [return: MarshalAs(UnmanagedType.Bool)]
    public static extern bool IsProcessorFeaturePresent(uint feature);
}
'@
    [AndroidPreflight]::Probe()
}
if ($results.checks.whpx.status -eq 'PASS' -and !$results.checks.whpx.value.available) {
    $results.checks.whpx.status = 'FAIL'
}
Record 'systemDpi' { [AndroidPreflight]::GetDpiForSystem() }
Record 'processorFeatures' {
    # These are diagnostic hints, not proof of firmware settings inside a VM
    # or when Hyper-V/VBS owns the virtualization extensions.
    @{
        secondLevelAddressTranslation = [AndroidPreflight]::IsProcessorFeaturePresent(20)
        virtualizationFirmwareEnabled = [AndroidPreflight]::IsProcessorFeaturePresent(21)
    }
}
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
    $files = @(@{ path = 'src-tauri/android-input/lomi-input.apk'; expected = $manifest.apkSha256 })
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
