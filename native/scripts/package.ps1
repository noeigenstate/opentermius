[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [string]$DumpbinPath,
    [string]$MakensisPath,
    [string]$WebView2BootstrapperPath
)
$ErrorActionPreference = 'Stop'
$nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
$installerRoot = Join-Path $nativeRoot 'installer'
$staging = Join-Path $installerRoot 'staging'
$releaseDirectory = Join-Path $nativeRoot 'releases'

function Find-Makensis([string]$ExplicitPath) {
    if ($ExplicitPath) {
        $candidate = $ExplicitPath
        if (Test-Path -LiteralPath $candidate -PathType Container) {
            $candidate = Join-Path $candidate 'makensis.exe'
        }
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
        throw "makensis.exe was not found at '$ExplicitPath'."
    }

    $commands = @(Get-Command 'makensis.exe' -CommandType Application -ErrorAction SilentlyContinue)
    if ($commands.Count -gt 0) { return $commands[0].Source }

    $standard = @()
    if (${env:ProgramFiles(x86)}) { $standard += (Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe') }
    if ($env:ProgramFiles) { $standard += (Join-Path $env:ProgramFiles 'NSIS\makensis.exe') }
    foreach ($candidate in $standard) {
        if (Test-Path -LiteralPath $candidate -PathType Leaf) {
            return (Resolve-Path -LiteralPath $candidate).Path
        }
    }

    if ($env:LOCALAPPDATA) {
        $cache = Join-Path $env:LOCALAPPDATA 'electron-builder\Cache\nsis'
        if (Test-Path -LiteralPath $cache -PathType Container) {
            $versions = Get-ChildItem -LiteralPath $cache -Directory -Filter 'nsis-*' |
                Sort-Object -Property Name -Descending
            foreach ($versionDirectory in $versions) {
                foreach ($relative in @('makensis.exe', 'Bin\makensis.exe')) {
                    $candidate = Join-Path $versionDirectory.FullName $relative
                    if (Test-Path -LiteralPath $candidate -PathType Leaf) {
                        return (Resolve-Path -LiteralPath $candidate).Path
                    }
                }
            }
        }
    }

    throw @'
NSIS (makensis.exe) was not found. Install it with:
  winget install -e --id NSIS.NSIS --accept-source-agreements --accept-package-agreements
or pass -MakensisPath <path to makensis.exe>.
'@
}

function Remove-OwnedDirectory([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path)) { return }
    $resolved = [IO.Path]::GetFullPath($Path)
    $rootPrefix = [IO.Path]::GetFullPath($nativeRoot).TrimEnd('\') + '\'
    if (-not $resolved.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Refusing to remove a directory outside the native workspace: $resolved"
    }
    Remove-Item -LiteralPath $resolved -Recurse -Force
}

$makensis = Find-Makensis $MakensisPath
Write-Host "Using NSIS: $makensis"

$version = & node (Join-Path $PSScriptRoot 'check-versions.mjs')
if ($LASTEXITCODE -ne 0) { throw 'Release versions do not match.' }
if ($env:CARGO_BUILD_TARGET -and $env:CARGO_BUILD_TARGET -ne 'x86_64-pc-windows-msvc') {
    throw 'The Windows x64 installer requires x86_64-pc-windows-msvc.'
}
if ($env:OPENTERMIUS_TEST_EXECUTABLE -and -not $SkipBuild) {
    throw 'OPENTERMIUS_TEST_EXECUTABLE is only supported with -SkipBuild.'
}

Push-Location $nativeRoot
try {
    if (-not $SkipBuild) {
        Push-Location (Join-Path $nativeRoot 'web')
        try {
            & npm ci
            if ($LASTEXITCODE -ne 0) { throw 'Frontend dependency installation failed.' }
            & npm run build
            if ($LASTEXITCODE -ne 0) { throw 'Frontend build failed.' }
        } finally { Pop-Location }
        # custom-protocol embeds web/dist; a plain cargo build uses the dev URL.
        & cargo build --release --locked -p opentermius-desktop --features tauri/custom-protocol
        if ($LASTEXITCODE -ne 0) { throw 'Release build failed.' }
    }
    $source = if ($env:OPENTERMIUS_TEST_EXECUTABLE) { [IO.Path]::GetFullPath($env:OPENTERMIUS_TEST_EXECUTABLE) } else {
        # Cargo prints UTF-8; Windows PowerShell 5.1 would decode non-ASCII paths with the ANSI code page.
        [Console]::OutputEncoding = [Text.UTF8Encoding]::new($false)
        $metadataJson = & cargo metadata --no-deps --format-version 1 --locked
        if ($LASTEXITCODE -ne 0) { throw 'Could not resolve the Cargo target directory.' }
        $targetRoot = ($metadataJson | ConvertFrom-Json).target_directory
        $target = if ($env:CARGO_BUILD_TARGET) { $env:CARGO_BUILD_TARGET } else { $null }
        $relative = if ($target) { "$target/release/opentermius.exe" } else { 'release/opentermius.exe' }
        $candidate = Join-Path $targetRoot $relative
        $candidate
    }
    if (-not (Test-Path -LiteralPath $source -PathType Leaf)) { throw 'Release executable is missing.' }
    $versionInfo = [Diagnostics.FileVersionInfo]::GetVersionInfo($source)
    if ($versionInfo.ProductName -ne 'OpenTermius' -or $versionInfo.FileDescription -ne 'OpenTermius' -or $versionInfo.CompanyName -ne 'OpenTermius') {
        throw 'Release executable does not contain the expected OpenTermius product identity.'
    }
    if ($versionInfo.ProductVersion -ne $version) {
        throw "Release executable version '$($versionInfo.ProductVersion)' does not match '$version'."
    }
    $image = [IO.File]::ReadAllBytes($source)
    $peOffset = [BitConverter]::ToInt32($image, 0x3c)
    if ([BitConverter]::ToUInt16($image, $peOffset + 4) -ne 0x8664) {
        throw 'Release executable is not a Windows x64 binary.'
    }

    Remove-OwnedDirectory $staging
    [void][IO.Directory]::CreateDirectory($staging)
    [void][IO.Directory]::CreateDirectory($releaseDirectory)
    Copy-Item -LiteralPath $source -Destination (Join-Path $staging 'opentermius.exe') -Force

    # Bundle Microsoft's small bootstrapper; only machines missing the runtime
    # need an internet connection during installation. No NSIS download plugin.
    $bootstrapper = Join-Path $staging 'MicrosoftEdgeWebview2Setup.exe'
    if ($WebView2BootstrapperPath) {
        Copy-Item -LiteralPath $WebView2BootstrapperPath -Destination $bootstrapper
    } else {
        [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
        Invoke-WebRequest -UseBasicParsing -TimeoutSec 120 -Uri 'https://go.microsoft.com/fwlink/p/?LinkId=2124703' -OutFile $bootstrapper
    }
    $signature = Get-AuthenticodeSignature -LiteralPath $bootstrapper
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
        throw 'The WebView2 bootstrapper must have a valid Microsoft signature.'
    }

    Push-Location $installerRoot
    try {
        & $makensis /V2 "/DVERSION=$version" 'installer.nsi'
        if ($LASTEXITCODE -ne 0) { throw "NSIS installer build failed with exit code $LASTEXITCODE." }
    } finally { Pop-Location }

    $setupName = "OpenTermius-$version-windows-x64-setup.exe"
    $setupPath = Join-Path $releaseDirectory $setupName
    if (-not (Test-Path -LiteralPath $setupPath -PathType Leaf)) {
        throw "NSIS did not produce the expected installer: $setupPath"
    }

    # The installer replaces the portable executable as the release deliverable.
    Get-ChildItem -LiteralPath $releaseDirectory -File -Filter 'OpenTermius-*-windows-x64.exe' |
        Remove-Item -Force
    Get-ChildItem -LiteralPath $releaseDirectory -File -Filter 'OpenTermius-*-windows-x64-setup.exe' |
        Where-Object { $_.Name -ne $setupName } |
        Remove-Item -Force

    $manifest = [ordered]@{
        version = $version
        product = $versionInfo.ProductName
        architecture = 'x86_64-pc-windows-msvc'
        installer = 'nsis'
        file = $setupName
        bytes = (Get-Item -LiteralPath $setupPath).Length
        sha256 = (Get-FileHash -LiteralPath $setupPath -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    if ($DumpbinPath) {
        $dumpbin = (Resolve-Path -LiteralPath $DumpbinPath).Path
        $dependencyOutput = & $dumpbin /dependents $source 2>&1
        if ($LASTEXITCODE -ne 0) { throw 'PE dependency inspection failed.' }
        $imports = @($dependencyOutput | ForEach-Object {
            if ($_ -match '^\s+([a-zA-Z0-9_.-]+\.dll)\s*$') { $Matches[1].ToLowerInvariant() }
        } | Sort-Object -Unique)
        if ($imports.Count -eq 0) { throw 'PE dependency inspection returned no DLL imports.' }
        $manifest.direct_dll_imports = $imports
        $manifest.imports_vc_runtime = @($imports | Where-Object { $_ -match '^(vcruntime|msvcp|msvcr).*\.dll$' }).Count -gt 0
    }
    $manifestPath = Join-Path $releaseDirectory 'checksums.json'
    [IO.File]::WriteAllText(
        $manifestPath,
        ($manifest | ConvertTo-Json) + [Environment]::NewLine,
        [Text.UTF8Encoding]::new($false)
    )
    $manifest | ConvertTo-Json
} finally {
    Pop-Location
    Remove-OwnedDirectory $staging
}
