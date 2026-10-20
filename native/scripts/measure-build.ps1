[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][ValidatePattern('^[a-zA-Z0-9_-]+$')][string]$Label,
    [ValidateRange(3, 20)][int]$Runs = 3,
    [ValidateSet('A', 'B', 'C', 'D', 'E')][string[]]$Scenarios = @('A', 'B', 'C', 'D', 'E'),
    [switch]$Lld
)
$ErrorActionPreference = 'Stop'
$nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$output = Join-Path $nativeRoot ("test-output/build-speed/{0}-{1}" -f $Label, (Get-Date -Format 'yyyyMMdd-HHmmss'))
[void][IO.Directory]::CreateDirectory($output)
$counterPath = Join-Path $output 'cpu.csv'
$samplesJob = Start-Job -ArgumentList $counterPath -ScriptBlock {
    param($path)
    $writer = [IO.StreamWriter]::new($path, $false, [Text.UTF8Encoding]::new($false))
    $writer.AutoFlush = $true
    try {
        Get-Counter '\Processor(_Total)\% Processor Time' -SampleInterval 1 -Continuous |
            ForEach-Object {
                $sample = $_.CounterSamples[0]
                $writer.WriteLine(('{0},{1}' -f [DateTime]::UtcNow.Ticks, $sample.CookedValue.ToString([Globalization.CultureInfo]::InvariantCulture)))
            }
    } finally { $writer.Dispose() }
}
$results = @()
function Invoke-MeasuredCargo([string[]]$CargoArguments, [string]$LogName) {
    $started = [DateTime]::UtcNow
    $timer = [Diagnostics.Stopwatch]::StartNew()
    # Cargo's machine-wide gate and file locks are intentionally included in wall time.
    $savedPreference = $ErrorActionPreference
    $ErrorActionPreference = 'Continue'
    & $env:ComSpec /d /c "cargo $($CargoArguments -join ' ') 2>&1" | Tee-Object -FilePath (Join-Path $output "$LogName.log") | Out-Host
    $code = $LASTEXITCODE
    $ErrorActionPreference = $savedPreference
    $timer.Stop()
    if ($code -ne 0) { throw "cargo $CargoArguments failed ($code); see $output" }
    $ended = [DateTime]::UtcNow
    $loads = @(Get-Content -LiteralPath $counterPath -ErrorAction SilentlyContinue | ForEach-Object {
        $parts = $_ -split ','
        if ($parts.Count -eq 2 -and [long]$parts[0] -ge $started.Ticks -and [long]$parts[0] -le $ended.Ticks) {
            [double]::Parse($parts[1], [Globalization.CultureInfo]::InvariantCulture)
        }
    })
    $stats = $loads | Measure-Object -Average -Maximum
    $record = [ordered]@{
        name = $LogName
        command = "cargo $($CargoArguments -join ' ')"
        startedUtc = $started.ToString('o')
        seconds = [Math]::Round($timer.Elapsed.TotalSeconds, 3)
        cpuSamples = $loads.Count
        cpuAverage = $stats.Average
        cpuMax = $stats.Maximum
        exitCode = $code
    }
    Write-Host ($record | ConvertTo-Json -Compress)
    return [pscustomobject]$record
}
Push-Location $nativeRoot
try {
    $buildArguments = @('build', '--locked')
    if ($Lld) {
        if (@($Scenarios | Where-Object { $_ -in @('D', 'E') }).Count -gt 0) {
            throw '-Lld measures A/B/C only; it changes the final binary linker without rebuilding dependencies.'
        }
        $sysroot = & rustc --print sysroot
        if ($LASTEXITCODE -ne 0) { throw 'Cannot find the Rust sysroot.' }
        $linkerPath = Join-Path $sysroot 'lib/rustlib/x86_64-pc-windows-msvc/bin/rust-lld.exe'
        if (-not (Test-Path -LiteralPath $linkerPath)) { throw "Missing linker: $linkerPath" }
        $buildArguments = @('rustc', '--locked', '--bin', 'opentermius', '--', '-C', "linker=`"$linkerPath`"", '-C', 'linker-flavor=lld-link')
    }
    $results += Invoke-MeasuredCargo $buildArguments 'warm-build'
    if (@($Scenarios | Where-Object { $_ -in @('D', 'E') }).Count -gt 0) {
        $results += Invoke-MeasuredCargo @('test', '--locked', '--no-run') 'warm-tests'
    }
    $touchPaths = @{ A = 'src/main.rs'; B = 'src/pages/settings.rs'; C = 'ui/main.slint'; D = 'src/store.rs' }
    foreach ($scenario in $Scenarios) {
        for ($run = 1; $run -le $Runs; $run++) {
            if ($scenario -ne 'E') {
                (Get-Item -LiteralPath (Join-Path $nativeRoot $touchPaths[$scenario])).LastWriteTimeUtc = [DateTime]::UtcNow
            }
            $cargoArguments = if ($scenario -eq 'D') { @('test', '--locked', '--no-run') } elseif ($scenario -eq 'E') { @('test', '--locked') } else { $buildArguments }
            $results += Invoke-MeasuredCargo $cargoArguments "$scenario-$run"
            [IO.File]::WriteAllText((Join-Path $output 'results.json'), (ConvertTo-Json -InputObject @($results) -Depth 5), [Text.UTF8Encoding]::new($false))
        }
    }
    Write-Host "Measurements: $output"
} finally {
    Pop-Location
    Stop-Job -Job $samplesJob
    Receive-Job -Job $samplesJob -ErrorAction Continue | Out-Host
    Remove-Job -Job $samplesJob
}
