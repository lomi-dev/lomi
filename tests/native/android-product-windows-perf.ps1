param(
    [Parameter(Mandatory = $true)][string]$ProductDirectory,
    [ValidateRange(1, 3600)][int]$DurationSeconds = 1800,
    [switch]$IncludeGpu,
    [ValidatePattern('^[A-Za-z0-9-]{1,100}$')][string]$EvidenceName = ('windows-perf-' + [DateTime]::UtcNow.ToString('yyyyMMddTHHmmss') + '-' + [Guid]::NewGuid().ToString('N'))
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version 2
if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) { throw 'Run on Windows.' }

function Directory-Path([string]$Path) {
    $item = Get-Item -LiteralPath $Path
    if (!$item.PSIsContainer) { throw "Not a directory: $Path" }
    # A junction/symlink must not redirect fixture evidence or its managed root.
    $ancestor = $item
    while ($null -ne $ancestor) {
        if (($ancestor.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw "Reparse point in fixture path: $($ancestor.FullName)" }
        $ancestor = $ancestor.Parent
    }
    return $item.FullName.TrimEnd([IO.Path]::DirectorySeparatorChar)
}
function Hash([string]$Path) { return (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant() }

$product = Directory-Path $ProductDirectory
$trial = Directory-Path (Split-Path -Parent $product)
if ((Split-Path -Leaf $product) -ne 'product' -or (Split-Path -Leaf $trial) -notlike 'lomi-android-stage0-*') {
    throw 'Use the product directory of an isolated lomi-android-stage0-* trial.'
}
$repository = (Resolve-Path (Join-Path $PSScriptRoot '../..')).Path.TrimEnd([IO.Path]::DirectorySeparatorChar)
if ($product -eq $repository -or $product.StartsWith($repository + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Performance evidence must be outside the repository.'
}
$receiptPath = Join-Path $product 'application.json'
$receiptFile = Get-Item -LiteralPath $receiptPath
if ($receiptFile.Length -gt 65536 -or ($receiptFile.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'Invalid application receipt.' }
$receiptHash = Hash $receiptPath
$receipt = Get-Content -Raw -LiteralPath $receiptPath | ConvertFrom-Json
$managed = Directory-Path $receipt.managed
if ((Split-Path -Parent $managed) -ne $trial -or (Split-Path -Leaf $managed) -notlike 'native-managed-*') { throw 'Managed directory belongs to another trial.' }
$consent = Get-Content -Raw -LiteralPath (Join-Path $trial 'evidence/consent.json') | ConvertFrom-Json
if ($consent.accepted -ne $true) { throw 'Missing isolated SDK consent.' }
if ($receipt.platform -ne 'win32' -or $receipt.architecture -ne 'x64' -or $receipt.buildProfile -ne 'debug' -or $receipt.identifier -notmatch '^dev\.lomi\.android-product-\d+$') {
    throw 'Not a Windows x64 debug product fixture receipt.'
}
if ((Directory-Path $receipt.repository) -ne $repository) { throw 'Receipt belongs to another repository.' }
$appPid = [int]$receipt.pid
if ($appPid -le 0 -or $receipt.binarySha256 -notmatch '^[a-fA-F0-9]{64}$') { throw 'Invalid application identity.' }
$expectedExe = [IO.Path]::GetFullPath((Join-Path $repository 'src-tauri/target/debug/lomi.exe'))
if ((Hash $expectedExe) -ne $receipt.binarySha256) { throw 'Application executable hash differs from receipt.' }
$exeFile = Get-Item -LiteralPath $expectedExe
$exeLength = $exeFile.Length
$exeWriteTicks = $exeFile.LastWriteTimeUtc.Ticks
$app = [Diagnostics.Process]::GetProcessById($appPid)
$null = $app.Handle # Retain the original process object/handle across PID reuse and exit.
$appCreated = $app.StartTime.ToUniversalTime()
$launchTime = [DateTimeOffset]::Parse($receipt.startedAt).UtcDateTime
if ([Math]::Abs(($appCreated - $launchTime).TotalSeconds) -gt 10) { throw 'Application creation time differs from receipt launch time.' }
$appSession = $app.SessionId
function Assert-Application {
    $app.Refresh()
    if ($app.HasExited -or $app.StartTime.ToUniversalTime().Ticks -ne $appCreated.Ticks -or $app.MainModule.FileName -ne $expectedExe -or $app.SessionId -ne $appSession) {
        throw 'Application stopped or its native identity changed.'
    }
    if ((Hash $receiptPath) -ne $receiptHash) { throw 'Application receipt changed during sampling.' }
    $file = Get-Item -LiteralPath $expectedExe
    if ($file.Length -ne $exeLength -or $file.LastWriteTimeUtc.Ticks -ne $exeWriteTicks) { throw 'Application executable changed during sampling.' }
}
Assert-Application

# Toolhelp is independent of WMI and requires no administrator or extra package.
Add-Type -TypeDefinition @'
using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Runtime.InteropServices;
public static class AndroidProductPerfSnapshot {
    [StructLayout(LayoutKind.Sequential, CharSet = CharSet.Unicode)]
    struct Entry {
        public uint size, usage, pid;
        public UIntPtr heap;
        public uint module, threads, parent;
        public int priority;
        public uint flags;
        [MarshalAs(UnmanagedType.ByValTStr, SizeConst = 260)] public string name;
    }
    public sealed class Item { public uint Pid, Parent; public string Name; }
    [DllImport("kernel32", SetLastError = true)] static extern IntPtr CreateToolhelp32Snapshot(uint flags, uint pid);
    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool Process32FirstW(IntPtr snapshot, ref Entry entry);
    [DllImport("kernel32", CharSet = CharSet.Unicode, SetLastError = true)] static extern bool Process32NextW(IntPtr snapshot, ref Entry entry);
    [DllImport("kernel32")] static extern bool CloseHandle(IntPtr handle);
    public static Item[] Read() {
        IntPtr snapshot = CreateToolhelp32Snapshot(2, 0);
        if (snapshot == new IntPtr(-1)) throw new Win32Exception(Marshal.GetLastWin32Error());
        try {
            var items = new List<Item>();
            var entry = new Entry(); entry.size = (uint)Marshal.SizeOf(typeof(Entry));
            bool available = Process32FirstW(snapshot, ref entry);
            while (available) {
                items.Add(new Item { Pid = entry.pid, Parent = entry.parent, Name = entry.name });
                available = Process32NextW(snapshot, ref entry);
            }
            int error = Marshal.GetLastWin32Error();
            if (error != 18) throw new Win32Exception(error);
            return items.ToArray();
        } finally { CloseHandle(snapshot); }
    }
}
public sealed class AndroidProductGpuQuery : System.IDisposable {
    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
    struct Value { public uint Status; public double Number; }
    [System.Runtime.InteropServices.StructLayout(System.Runtime.InteropServices.LayoutKind.Sequential)]
    struct Item { public System.IntPtr Name; public Value Value; }
    public sealed class Sample { public string Instance; public uint Status; public double UtilizationPercentage; }
    [System.Runtime.InteropServices.DllImport("pdh.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    static extern uint PdhOpenQueryW(string source, System.UIntPtr user, out System.IntPtr query);
    [System.Runtime.InteropServices.DllImport("pdh.dll", CharSet = System.Runtime.InteropServices.CharSet.Unicode)]
    static extern uint PdhAddEnglishCounterW(System.IntPtr query, string path, System.UIntPtr user, out System.IntPtr counter);
    [System.Runtime.InteropServices.DllImport("pdh.dll")]
    static extern uint PdhCollectQueryData(System.IntPtr query);
    [System.Runtime.InteropServices.DllImport("pdh.dll")]
    static extern uint PdhGetFormattedCounterArrayW(System.IntPtr counter, uint format, ref uint bytes, out uint count, System.IntPtr buffer);
    [System.Runtime.InteropServices.DllImport("pdh.dll")]
    static extern uint PdhCloseQuery(System.IntPtr query);
    System.IntPtr query, counter;
    static void Check(uint status) { if (status != 0) throw new System.InvalidOperationException("GPU Engine PDH query failed (0x" + status.ToString("X8") + ")."); }
    public AndroidProductGpuQuery() {
        Check(PdhOpenQueryW(null, System.UIntPtr.Zero, out query));
        try {
            Check(PdhAddEnglishCounterW(query, @"\GPU Engine(*)\Utilization Percentage", System.UIntPtr.Zero, out counter));
            Check(PdhCollectQueryData(query));
        } catch { Dispose(); throw; }
    }
    public Sample[] Read() {
        Check(PdhCollectQueryData(query));
        uint bytes = 0, count;
        uint status = PdhGetFormattedCounterArrayW(counter, 0x8200, ref bytes, out count, System.IntPtr.Zero);
        if (status != 0x800007D2) Check(status); // PDH_MORE_DATA
        if (bytes == 0 || bytes > 16 * 1024 * 1024) throw new System.InvalidOperationException("Unexpected GPU counter buffer size.");
        System.IntPtr buffer = System.Runtime.InteropServices.Marshal.AllocHGlobal(checked((int)bytes));
        try {
            Check(PdhGetFormattedCounterArrayW(counter, 0x8200, ref bytes, out count, buffer));
            int stride = System.Runtime.InteropServices.Marshal.SizeOf(typeof(Item));
            if ((ulong)count * (ulong)stride > bytes) throw new System.InvalidOperationException("Unexpected GPU counter item count.");
            var samples = new Sample[count];
            for (int index = 0; index < samples.Length; index++) {
                var item = (Item)System.Runtime.InteropServices.Marshal.PtrToStructure(System.IntPtr.Add(buffer, checked(index * stride)), typeof(Item));
                samples[index] = new Sample {
                    Instance = System.Runtime.InteropServices.Marshal.PtrToStringUni(item.Name),
                    Status = item.Value.Status, UtilizationPercentage = item.Value.Number
                };
            }
            return samples;
        } finally { System.Runtime.InteropServices.Marshal.FreeHGlobal(buffer); }
    }
    public void Dispose() { if (query != System.IntPtr.Zero) { PdhCloseQuery(query); query = System.IntPtr.Zero; } }
}
'@

$output = Join-Path $product ($EvidenceName + '.jsonl')
# CreateNew atomically refuses any previous evidence, including a pre-existing link.
$stream = [IO.File]::Open($output, [IO.FileMode]::CreateNew, [IO.FileAccess]::Write, [IO.FileShare]::Read)
$writer = [IO.StreamWriter]::new($stream, [Text.UTF8Encoding]::new($false))
$writer.AutoFlush = $true
$tracked = @{}
$discoveryFailures = @{}
$gpuState = @{ query = $null; previousSeconds = 0.0; previousIdentities = @{}; unavailable = $false }
$watch = [Diagnostics.Stopwatch]::StartNew()
$started = [DateTime]::UtcNow
function Write-Record($Record) { $writer.WriteLine(($Record | ConvertTo-Json -Depth 8 -Compress)) }
function Track($Process, [int]$Parent, [bool]$Initial) {
    $null = $Process.Handle
    $created = $Process.StartTime.ToUniversalTime()
    $key = '{0}:{1}' -f $Process.Id, $created.Ticks
    if ($tracked.ContainsKey($key)) { $Process.Dispose(); return }
    $cpu = $Process.TotalProcessorTime.TotalSeconds
    $baseline = 0.0
    if ($Initial -or $created -lt $started) { $baseline = $cpu }
    $tracked[$key] = [pscustomobject]@{
        process = $Process; pid = $Process.Id; parent = $Parent; created = $created;
        exe = $Process.MainModule.FileName; baseline = $baseline; cpu = $cpu;
        exited = $false; finalCpuUnavailable = $false
    }
    Write-Record ([ordered]@{ type = 'process'; event = 'included'; pid = $Process.Id; parentPid = $Parent; createdAt = $created.ToString('o'); executable = $Process.MainModule.FileName; baselineCpuSeconds = $baseline })
}
function Discover([bool]$Initial) {
    $snapshot = @([AndroidProductPerfSnapshot]::Read())
    # Traverse only verified WebView2 branches. Emulator, QEMU, adb, Java and
    # unrelated descendants cannot enter the accounting set or authorize children.
    $parents = @{}
    foreach ($entry in @($tracked.Values)) {
        $entry.process.Refresh()
        if (!$entry.process.HasExited -and $entry.process.StartTime.ToUniversalTime().Ticks -eq $entry.created.Ticks) { $parents[$entry.pid] = $entry.created }
    }
    $seen = @{}
    for ($depth = 0; $depth -lt 32; $depth++) {
        $added = $false
        foreach ($candidate in $snapshot) {
            $childPid = [int]$candidate.Pid
            $parentPid = [int]$candidate.Parent
            if ($seen.ContainsKey($childPid) -or !$parents.ContainsKey($parentPid) -or $candidate.Name -ine 'msedgewebview2.exe') { continue }
            $seen[$childPid] = $true
            $child = $null
            $stage = 'open-child'
            try {
                $child = [Diagnostics.Process]::GetProcessById($childPid)
                $stage = 'verify-child'
                $null = $child.Handle
                $created = $child.StartTime.ToUniversalTime()
                $child.Refresh()
                if ($child.HasExited) {
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-exited'; pid = $childPid; parentPid = $parentPid })
                    continue
                }
                # Pin the candidate first, then verify a new snapshot still
                # attributes that live identity to the authorizing parent.
                $current = @([AndroidProductPerfSnapshot]::Read() | Where-Object { $_.Pid -eq $childPid })
                $child.Refresh()
                if ($child.HasExited) {
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-exited'; pid = $childPid; parentPid = $parentPid })
                    continue
                }
                if ($current.Count -ne 1) { throw 'A live pinned WebView2 candidate is absent or ambiguous in the current snapshot.' }
                if ($current[0].Parent -ne $parentPid -or $current[0].Name -ine 'msedgewebview2.exe' -or $child.StartTime.ToUniversalTime().Ticks -ne $created.Ticks) {
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-stale'; pid = $childPid; parentPid = $parentPid; reason = 'Pinned child no longer matches the snapshot parent or native identity.' })
                    continue
                }
                if ($child.SessionId -ne $appSession -or $created -lt $parents[$parentPid] -or [IO.Path]::GetFileName($child.MainModule.FileName) -ine 'msedgewebview2.exe') { throw 'WebView2 child identity mismatch.' }
                # Refresh the authorizing parent after opening the child, so a
                # reused PID in a Toolhelp snapshot cannot establish ownership.
                $owner = [Diagnostics.Process]::GetProcessById($parentPid)
                try {
                    $null = $owner.Handle
                    if ($owner.HasExited -or $owner.StartTime.ToUniversalTime().Ticks -ne $parents[$parentPid].Ticks) {
                        Write-Record ([ordered]@{ type = 'process'; event = 'discovery-stale'; pid = $childPid; parentPid = $parentPid; reason = 'Authorizing parent exited or its PID was reused.' })
                        continue
                    }
                } finally { $owner.Dispose() }
                $child.Refresh()
                if ($child.HasExited) {
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-exited'; pid = $childPid; parentPid = $parentPid })
                    continue
                }
                if ($child.StartTime.ToUniversalTime().Ticks -ne $created.Ticks) { throw 'Pinned WebView2 creation identity changed.' }
                Track $child $parentPid $Initial
                $child = $null # The tracked record owns the process handle.
                $parents[$childPid] = $created
                $added = $true
            } catch {
                $failure = $_.Exception
                $confirmedExited = ($stage -eq 'open-child') -and ($failure.GetBaseException() -is [ArgumentException])
                if ($null -ne $child) {
                    try { $child.Refresh(); $confirmedExited = $child.HasExited } catch { }
                }
                if ($confirmedExited) {
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-exited'; pid = $childPid; parentPid = $parentPid })
                } else {
                    # A later successful read cannot recover CPU/RAM samples
                    # omitted while this live branch was inaccessible.
                    $discoveryFailures[$childPid] = $failure.Message
                    Write-Record ([ordered]@{ type = 'process'; event = 'discovery-unavailable'; pid = $childPid; parentPid = $parentPid; error = $failure.Message })
                }
            } finally { if ($null -ne $child) { $child.Dispose() } }
        }
        if (!$added) { break }
    }
}
function Gpu-Identities {
    $identities = @{}
    foreach ($entry in @($tracked.Values)) {
        $entry.process.Refresh()
        if (!$entry.process.HasExited -and $entry.process.StartTime.ToUniversalTime().Ticks -eq $entry.created.Ticks) { $identities[$entry.pid] = $entry }
    }
    return $identities
}
function Write-GpuSample {
    try {
        $identities = Gpu-Identities
        $values = @($gpuState.query.Read())
        $now = $watch.Elapsed.TotalSeconds
        $raw = @(); $engines = @{}; $warmingUp = @{}
        foreach ($value in $values) {
            if ($value.Instance -notmatch '^pid_(?<processId>\d+)_(?<engine>luid_0x[0-9a-f]+_0x[0-9a-f]+_phys_\d+_eng_\d+_engtype_.+)$') { continue }
            $enginePid = [int]$Matches.processId
            $engine = $Matches.engine.ToLowerInvariant()
            if (!$identities.ContainsKey($enginePid)) { continue }
            $entry = $identities[$enginePid]
            # PDH names contain only a PID. Retain the native handle and check
            # its creation identity on both sides of counter collection.
            $entry.process.Refresh()
            if ($entry.process.HasExited -or $entry.process.StartTime.ToUniversalTime().Ticks -ne $entry.created.Ticks) { continue }
            # A formatted PDH rate pairs raw values by instance name, which
            # contains only the PID. Require the same pinned creation identity
            # at the preceding collection before trusting this interval.
            $stable = $gpuState.previousIdentities.ContainsKey($enginePid) -and $gpuState.previousIdentities[$enginePid].created.Ticks -eq $entry.created.Ticks
            $intervalStatus = 'available'
            if (!$stable) { $warmingUp[$enginePid] = $true; $intervalStatus = 'warming-up' }
            $valid = $stable -and ($value.Status -eq 0 -or $value.Status -eq 1) -and ![double]::IsNaN($value.UtilizationPercentage) -and ![double]::IsInfinity($value.UtilizationPercentage) -and $value.UtilizationPercentage -ge 0
            $utilization = $null
            if ($valid) { $utilization = $value.UtilizationPercentage }
            if ($stable -and !$valid) { $intervalStatus = 'unavailable' }
            $raw += [pscustomobject]@{ pid = $enginePid; createdAt = $entry.created.ToString('o'); instance = $value.Instance; engine = $engine; pdhStatus = ('0x{0:X8}' -f $value.Status); intervalStatus = $intervalStatus; utilizationPercentage = $utilization }
            if (!$valid) { $gpuState.unavailable = $true; continue }
            if (!$engines.ContainsKey($engine)) { $engines[$engine] = 0.0 }
            $engines[$engine] += $value.UtilizationPercentage
        }
        $perEngine = @(); $maximum = 0.0
        foreach ($engine in @($engines.Keys | Sort-Object)) {
            $perEngine += [pscustomobject]@{ engine = $engine; cohortUtilizationPercentage = $engines[$engine] }
            $maximum = [Math]::Max($maximum, $engines[$engine])
        }
        $status = 'available'
        $fraction = $null
        if ($raw.Count -eq 0 -or $engines.Count -eq 0 -or @($raw | Where-Object { $null -eq $_.utilizationPercentage }).Count -gt 0) {
            $gpuState.unavailable = $true
            $status = 'unavailable'
        } else { $fraction = [Math]::Min(1.0, $maximum / 100.0) }
        if ($warmingUp.Count -gt 0) { $status = 'incomplete-warmup' }
        Write-Record ([ordered]@{ type = 'gpu-sample'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $now; intervalSeconds = ($now - $gpuState.previousSeconds); status = $status; warmingUpPids = @($warmingUp.Keys); rawProcessEngines = $raw; perEngine = $perEngine; maxEngineBusyFraction = $fraction })
        $gpuState.previousSeconds = $now
        $gpuState.previousIdentities = $identities
    } catch {
        $gpuState.unavailable = $true
        $gpuState.previousIdentities = @{}
        Write-Record ([ordered]@{ type = 'gpu-sample'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds; status = 'unavailable'; error = $_.Exception.Message; maxEngineBusyFraction = $null })
    }
}
function Counters {
    $cpu = 0.0; $working = [long]0; $private = [long]0
    $liveIds = @(); $finalUnavailable = @()
    foreach ($entry in @($tracked.Values)) {
        $process = $entry.process
        $process.Refresh()
        $exited = $process.HasExited
        try { $entry.cpu = [Math]::Max($entry.cpu, $process.TotalProcessorTime.TotalSeconds) }
        catch {
            if (!$exited) { throw }
            $entry.finalCpuUnavailable = $true
        }
        # An integer zero selects an integer overload and rounds each process.
        $cpu += [Math]::Max(0.0, [double]($entry.cpu - $entry.baseline))
        if (!$exited) {
            try {
                $working += $process.WorkingSet64
                $private += $process.PrivateMemorySize64
                $liveIds += $entry.pid
            } catch { if (!$process.HasExited) { throw } }
        }
        if ($exited -and !$entry.exited) {
            Write-Record ([ordered]@{ type = 'process'; event = 'exited'; pid = $entry.pid; createdAt = $entry.created.ToString('o'); retainedCpuSeconds = $entry.cpu; finalCpuUnavailable = $entry.finalCpuUnavailable })
            $entry.exited = $true
        }
        if ($entry.finalCpuUnavailable) { $finalUnavailable += $entry.pid }
    }
    return [pscustomobject]@{ cpu = $cpu; working = $working; private = $private; ids = $liveIds; finalUnavailable = $finalUnavailable }
}
try {
    Write-Record ([ordered]@{
        type = 'metadata'; startedAt = $started.ToString('o'); durationSeconds = $DurationSeconds; intervalSeconds = 2;
        applicationPid = $appPid; applicationCreatedAt = $appCreated.ToString('o'); receiptStartedAt = $receipt.startedAt; launchTimeToleranceSeconds = 10; executable = $expectedExe;
        binarySha256 = $receipt.binarySha256; receiptSha256 = $receiptHash; managed = $managed; product = $product;
        metricScope = 'application and verified msedgewebview2 descendants only; excludes emulator/adb/guest';
        gpu = [ordered]@{
            requested = $IncludeGpu.IsPresent; counter = '\GPU Engine(*)\Utilization Percentage';
            interpretation = 'PDH interval utilization attributed to the verified process cohort, summed only within each adapter/physical-engine identity; maxEngineBusyFraction is the largest engine total divided by 100, capped at 1. Raw process/engine readings and uncapped engine totals are retained. Engines are never summed together.';
            limitations = 'A busiest-engine utilization proxy, not the macOS union of active GPU intervals and not a calibrated equivalent of its 0.10 s/s budget. Driver scheduling, overlapping engines and adapters differ. No FPS or frame latency is implied; exited processes and engines absent between collections may be missed. GPU rates require the same pinned PID and creation identity at both collections; a new identity produces an incomplete warmup interval, never zero usage. Compare baseline and visible intervals with the same viewport/workload using capturedAt and the phase evidence.'
        };
        limitations = @('No framebuffer, FPS, IPC, animation or GPU metrics; not qualification by itself.', 'Children that start and exit between snapshots may be missed.', 'Exited child CPU retains the final readable value; unavailable final reads are marked.', 'Executable SHA256 checked at start/end; file length and write time and process creation checked each sample.')
    })
    Track $app 0 $true
    Discover $true
    if ($IncludeGpu) {
        $gpuState.previousIdentities = Gpu-Identities
        $gpuState.query = [AndroidProductGpuQuery]::new()
        $gpuState.previousSeconds = $watch.Elapsed.TotalSeconds
        Write-Record ([ordered]@{ type = 'gpu-sample'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $gpuState.previousSeconds; status = 'warming-up'; maxEngineBusyFraction = $null })
    }
    $previous = Counters
    $previousTime = $watch.Elapsed.TotalSeconds
    Write-Record ([ordered]@{ type = 'sample'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $previousTime; cpuCores = $null; cumulativeCpuSeconds = $previous.cpu; workingSetBytes = $previous.working; privateBytes = $previous.private; livePids = @($previous.ids); finalCpuUnavailablePids = @($previous.finalUnavailable) })
    while ($watch.Elapsed.TotalSeconds -lt $DurationSeconds) {
        $sleep = [Math]::Min(2.0, $DurationSeconds - $watch.Elapsed.TotalSeconds)
        if ($sleep -gt 0) { Start-Sleep -Milliseconds ([int][Math]::Ceiling($sleep * 1000)) }
        Assert-Application
        Discover $false
        $current = Counters
        $now = $watch.Elapsed.TotalSeconds
        $interval = $now - $previousTime
        Write-Record ([ordered]@{ type = 'sample'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $now; intervalSeconds = $interval; cpuCores = (($current.cpu - $previous.cpu) / $interval); cumulativeCpuSeconds = $current.cpu; workingSetBytes = $current.working; privateBytes = $current.private; livePids = @($current.ids); finalCpuUnavailablePids = @($current.finalUnavailable) })
        $previous = $current; $previousTime = $now
        if ($IncludeGpu) { Write-GpuSample }
    }
    Assert-Application
    if ((Hash $expectedExe) -ne $receipt.binarySha256) { throw 'Application executable hash changed during sampling.' }
    if ($discoveryFailures.Count -gt 0) {
        Write-Record ([ordered]@{ type = 'incomplete'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds; reason = 'WebView2 cohort discovery was unavailable during sampling.'; discoveryUnavailablePids = @($discoveryFailures.Keys); qualification = $false })
        throw 'WebView2 cohort discovery was incomplete; CPU/RAM samples may omit browser processes.'
    }
    $liveBrowser = $false
    foreach ($entry in @($tracked.Values)) {
        if ($entry.pid -eq $appPid) { continue }
        $entry.process.Refresh()
        if (!$entry.process.HasExited -and $entry.process.StartTime.ToUniversalTime().Ticks -eq $entry.created.Ticks) { $liveBrowser = $true }
    }
    if (!$liveBrowser) {
        Write-Record ([ordered]@{ type = 'incomplete'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds; reason = 'No live verified WebView2 branch remains.'; qualification = $false })
        throw 'Sampling requires a live verified WebView2 branch.'
    }
    if ($IncludeGpu -and $gpuState.unavailable) {
        Write-Record ([ordered]@{ type = 'incomplete'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds; reason = 'Requested GPU Engine samples were unavailable or invalid.'; qualification = $false })
        throw 'Requested GPU Engine sampling was incomplete.'
    }
    Write-Record ([ordered]@{ type = 'complete'; capturedAt = [DateTime]::UtcNow.ToString('o'); elapsedSeconds = $watch.Elapsed.TotalSeconds; binarySha256Verified = $true; gpuRequested = $IncludeGpu.IsPresent; qualification = $false })
} catch {
    Write-Record ([ordered]@{ type = 'error'; capturedAt = [DateTime]::UtcNow.ToString('o'); error = $_.Exception.Message })
    throw
} finally {
    if ($null -ne $gpuState.query) { $gpuState.query.Dispose() }
    foreach ($entry in @($tracked.Values)) { $entry.process.Dispose() }
    $app.Dispose()
    $writer.Dispose()
}
Write-Output $output
