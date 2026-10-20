[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ExecutablePath,
    [string[]]$ArgumentList = @(),
    [ValidateRange(1, 300)][int]$SampleSeconds = 5,
    [ValidateRange(0, 120)][int]$WarmupSeconds = 3,
    [string]$ResultFile,
    [string]$Label = 'application',
    [string]$ReadyFile,
    [switch]$StartupOnly
)

$ErrorActionPreference = 'Stop'
$executable = (Resolve-Path -LiteralPath $ExecutablePath).Path
$tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath())
$dataDirectory = Join-Path $tempRoot ('opentermius-benchmark-' + [Guid]::NewGuid().ToString('N'))
[void][IO.Directory]::CreateDirectory($dataDirectory)
$arguments = @($ArgumentList | ForEach-Object { $_.Replace('{data}', $dataDirectory) })
$tracked = @{}
$samples = [Collections.Generic.List[object]]::new()
$clock = [Diagnostics.Stopwatch]::new()
$process = $null
$readyMilliseconds = $null

function Register-Process([Diagnostics.Process]$Candidate) {
    try {
        $tracked[[int]$Candidate.Id] = @{
            StartTicks = $Candidate.StartTime.ToUniversalTime().Ticks
            CpuSeconds = $Candidate.TotalProcessorTime.TotalSeconds
        }
    } catch { } # A short-lived child can exit between discovery and inspection.
}

function Update-ProcessTree {
    $rows = @(Get-CimInstance Win32_Process -Property ProcessId, ParentProcessId, CreationDate)
    $changed = $true
    while ($changed) {
        $changed = $false
        foreach ($row in $rows) {
            $childId = [int]$row.ProcessId
            if ($tracked.ContainsKey($childId) -or -not $tracked.ContainsKey([int]$row.ParentProcessId)) { continue }
            $parentIdentity = $tracked[[int]$row.ParentProcessId]
            # Creation identity prevents attaching a child of an unrelated reused PID.
            if ($row.CreationDate.ToUniversalTime().Ticks -lt $parentIdentity.StartTicks) { continue }
            try {
                $parent = Get-Process -Id $row.ParentProcessId -ErrorAction Stop
                if ($parent.StartTime.ToUniversalTime().Ticks -ne $parentIdentity.StartTicks) { continue }
                $candidate = Get-Process -Id $childId -ErrorAction Stop
                Register-Process $candidate
                if ($tracked.ContainsKey($childId)) { $changed = $true }
            } catch { }
        }
    }
}

function Read-Sample {
    Update-ProcessTree
    $workingSet = [long]0
    $privateBytes = [long]0
    $liveCount = 0
    foreach ($entry in @($tracked.GetEnumerator())) {
        try {
            $candidate = Get-Process -Id $entry.Key -ErrorAction Stop
            if ($candidate.StartTime.ToUniversalTime().Ticks -ne $entry.Value.StartTicks) { continue }
            $candidate.Refresh()
            $workingSet += $candidate.WorkingSet64
            $privateBytes += $candidate.PrivateMemorySize64
            $entry.Value.CpuSeconds = $candidate.TotalProcessorTime.TotalSeconds
            $liveCount++
        } catch { }
    }
    $cpuSeconds = [double]0
    foreach ($identity in $tracked.Values) { $cpuSeconds += $identity.CpuSeconds }
    [pscustomobject]@{ ElapsedSeconds = $clock.Elapsed.TotalSeconds; WorkingSetBytes = $workingSet; PrivateBytes = $privateBytes; ProcessCount = $liveCount; CpuSeconds = $cpuSeconds }
}

try {
    $clock.Start()
    $launch = @{ FilePath = $executable; PassThru = $true; WindowStyle = 'Hidden' }
    if ($arguments.Count -gt 0) {
        # Start-Process joins arguments into a command line; quote each literal safely.
        $launch.ArgumentList = @($arguments | ForEach-Object {
            if ($_ -match '[\s"]') { '"' + ([regex]::Replace($_, '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"' } else { $_ }
        })
    }
    $process = Start-Process @launch
    Register-Process $process
    $readinessPath = $null
    if ($ReadyFile) {
        $readinessPath = [IO.Path]::GetFullPath((Join-Path $dataDirectory $ReadyFile))
        if (-not $readinessPath.StartsWith($dataDirectory + [IO.Path]::DirectorySeparatorChar, [StringComparison]::OrdinalIgnoreCase)) { throw 'ReadyFile must stay inside the isolated benchmark data directory.' }
        while (-not (Test-Path -LiteralPath $readinessPath) -and $clock.Elapsed.TotalSeconds -lt 45) {
            $process.Refresh()
            if ($process.HasExited) { break }
            Start-Sleep -Milliseconds 20
        }
        if (Test-Path -LiteralPath $readinessPath) { $readyMilliseconds = [Math]::Round($clock.Elapsed.TotalMilliseconds, 1) }
        else { throw 'Application readiness marker was not written.' }
    }
    if (-not $StartupOnly) {
        while ($clock.Elapsed.TotalSeconds -lt $WarmupSeconds) { Start-Sleep -Milliseconds 100 }
        $initial = Read-Sample
        if ($initial.ProcessCount -eq 0) { throw 'Application exited before the measurement interval.' }
        $samples.Add($initial)
        while ($clock.Elapsed.TotalSeconds -lt ($initial.ElapsedSeconds + $SampleSeconds)) {
            Start-Sleep -Milliseconds 250
            $samples.Add((Read-Sample))
        }
        $last = $samples[$samples.Count - 1]
        if ($last.ProcessCount -eq 0) { throw 'Application exited during the measurement interval.' }
        $cpuDelta = $last.CpuSeconds - $initial.CpuSeconds
        $duration = $last.ElapsedSeconds - $initial.ElapsedSeconds
    }
    $result = [ordered]@{
        SchemaVersion = 1
        Label = $Label
        MeasuredAtUtc = [DateTime]::UtcNow.ToString('o')
        ExecutableBytes = (Get-Item -LiteralPath $executable).Length
        LogicalProcessors = [Environment]::ProcessorCount
        Isolation = 'fresh temporary data directory; test process tree only'
        StartupReadyMilliseconds = $readyMilliseconds
        StartupCriterion = $(if ($ReadyFile) { 'application-written readiness marker, polled every 20 ms; includes process launch' } else { 'not measured' })
    }
    if (-not $StartupOnly) {
        $result.WarmupSeconds = $WarmupSeconds
        $result.SampleDurationSeconds = [Math]::Round($duration, 3)
        $result.SampleCount = $samples.Count
        $result.ProcessCount = $last.ProcessCount
        $result.WorkingSetMeanBytes = [long](($samples | Measure-Object -Property WorkingSetBytes -Average).Average)
        $result.WorkingSetPeakBytes = [long](($samples | Measure-Object -Property WorkingSetBytes -Maximum).Maximum)
        $result.PrivateBytesMean = [long](($samples | Measure-Object -Property PrivateBytes -Average).Average)
        $result.CpuSeconds = [Math]::Round($cpuDelta, 4)
        $result.CpuPercentOneCore = [Math]::Round(100 * $cpuDelta / $duration, 3)
        $result.CpuPercentMachine = [Math]::Round(100 * $cpuDelta / $duration / [Environment]::ProcessorCount, 3)
    }
    $json = $result | ConvertTo-Json -Depth 6
    if ($ResultFile) {
        $resultPath = [IO.Path]::GetFullPath($ResultFile)
        [void][IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($resultPath))
        [IO.File]::WriteAllText($resultPath, $json + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    }
    $json
} finally {
    if ($process) {
        try { Update-ProcessTree } catch { }
        # Stop root first so it cannot create more children, then only known identities.
        $ids = @($process.Id) + @($tracked.Keys | Where-Object { $_ -ne $process.Id })
        foreach ($processId in $ids) {
            if (-not $tracked.ContainsKey([int]$processId)) { continue }
            try {
                $candidate = Get-Process -Id $processId -ErrorAction Stop
                if ($candidate.StartTime.ToUniversalTime().Ticks -eq $tracked[[int]$processId].StartTicks) { $candidate.Kill(); $candidate.WaitForExit(3000) | Out-Null }
            } catch { }
        }
    }
    $resolvedData = [IO.Path]::GetFullPath($dataDirectory)
    if ($resolvedData.StartsWith($tempRoot.TrimEnd('\') + '\', [StringComparison]::OrdinalIgnoreCase) -and [IO.Path]::GetFileName($resolvedData) -match '^opentermius-benchmark-[a-f0-9]{32}$') {
        Remove-Item -LiteralPath $resolvedData -Recurse -Force -ErrorAction SilentlyContinue
    } else { throw 'Refusing cleanup outside the verified benchmark temporary directory.' }
}
