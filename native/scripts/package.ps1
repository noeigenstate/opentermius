[CmdletBinding()]
param([switch]$SkipBuild, [string]$DumpbinPath)
$ErrorActionPreference = 'Stop'
$nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
Push-Location $nativeRoot
try {
    if (-not $SkipBuild) {
        & cargo build --release --locked
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $source = Join-Path $nativeRoot 'target/x86_64-pc-windows-msvc/release/opentermius.exe'
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Release executable is missing.' }
    $versionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($source)
    if ($versionInfo.ProductName -ne 'OpenTermius' -or $versionInfo.InternalName -ne 'opentermius') {
        throw 'Release executable does not contain the expected OpenTermius product identity.'
    }
    $releaseDirectory = Join-Path $nativeRoot 'releases'
    [void][IO.Directory]::CreateDirectory($releaseDirectory)
    $destination = Join-Path $releaseDirectory 'OpenTermius-0.2.0-windows-x64.exe'
    Copy-Item -LiteralPath $source -Destination $destination -Force
    $manifest = [ordered]@{
        version = '0.2.0'
        product = $versionInfo.ProductName
        architecture = 'x86_64-pc-windows-msvc'
        file = [IO.Path]::GetFileName($destination)
        bytes = (Get-Item -LiteralPath $destination).Length
        sha256 = (Get-FileHash -LiteralPath $destination -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($DumpbinPath) {
        $dumpbin = (Resolve-Path -LiteralPath $DumpbinPath).Path
        $dependencyOutput = & $dumpbin /dependents $destination 2>&1
        if ($LASTEXITCODE -ne 0) { throw 'PE dependency inspection failed.' }
        $imports = @($dependencyOutput | ForEach-Object {
            if ($_ -match '^\s+([a-zA-Z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
        } | Sort-Object -Unique)
        if ($imports.Count -eq 0) { throw 'PE dependency inspection returned no DLL imports.' }
        $manifest.direct_dll_imports = $imports
        $manifest.imports_vc_runtime = @($imports | Where-Object { $_ -match '^(vcruntime|msvcp|msvcr).*\.dll$' }).Count -gt 0
    }
    $manifestPath = Join-Path $releaseDirectory 'checksums.json'
    [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json) + [Environment]::NewLine, [Text.UTF8Encoding]::new($false))
    $manifest | ConvertTo-Json
} finally { Pop-Location }
