[CmdletBinding()]
param(
    [string]$SetupPath,
    [switch]$KeepInstalled
)
$ErrorActionPreference = 'Stop'

$nativeRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..'))
if (-not $SetupPath) {
    $SetupPath = Get-ChildItem -LiteralPath (Join-Path $nativeRoot 'releases') -File -Filter 'OpenTermius-*-windows-x64-setup.exe' |
        Sort-Object -Property LastWriteTimeUtc -Descending |
        Select-Object -First 1 -ExpandProperty FullName
}
if (-not $SetupPath -or -not (Test-Path -LiteralPath $SetupPath -PathType Leaf)) {
    throw 'Installer not found. Run scripts/package.ps1 first or pass -SetupPath.'
}
$setup = (Resolve-Path -LiteralPath $SetupPath).Path

$version = & node (Join-Path $nativeRoot 'scripts/check-versions.mjs')
if ($LASTEXITCODE -ne 0) { throw 'Release versions do not match.' }

$appDataDir = Join-Path $env:APPDATA 'OpenTermius'
$configExisted = Test-Path -LiteralPath $appDataDir -PathType Container
$sentinel = Join-Path $appDataDir ('installer-test-' + [Guid]::NewGuid().ToString('N') + '.txt')
$installDir = Join-Path ([IO.Path]::GetTempPath()) ('OpenTermius installer test-' + [Guid]::NewGuid().ToString('N'))
$programs = [Environment]::GetFolderPath('Programs')
$desktop = [Environment]::GetFolderPath('Desktop')
$startMenuShortcut = Join-Path $programs 'OpenTermius.lnk'
$desktopShortcut = Join-Path $desktop 'OpenTermius.lnk'
$uninstallKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\OpenTermius'
$appKey = 'HKCU:\Software\OpenTermius'

$results = [ordered]@{}

function Assert-True([bool]$Condition, [string]$Message) {
    if (-not $Condition) { throw "Assertion failed: $Message" }
}

function Wait-Condition([scriptblock]$Condition, [string]$Description, [int]$TimeoutSeconds = 60) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        if (& $Condition) { return }
        Start-Sleep -Milliseconds 250
    }
    throw "Timed out waiting for $Description."
}

function Invoke-Setup([string[]]$Arguments, [string]$Description) {
    $process = Start-Process -FilePath $setup -ArgumentList $Arguments -Wait -PassThru -WindowStyle Hidden
    if ($process.ExitCode -ne 0) {
        $hint = ''
        if ($process.ExitCode -eq 2) {
            $hint = ' The installer reported that OpenTermius is running; close it and retry.'
        }
        throw "$Description failed with exit code $($process.ExitCode).$hint"
    }
}

function Invoke-Uninstaller([string]$Uninstaller, [string[]]$Arguments) {
    $process = Start-Process -FilePath $Uninstaller -ArgumentList $Arguments -Wait -PassThru -WindowStyle Hidden
    return $process.ExitCode
}

$preExisting = @(
    (Test-Path -LiteralPath $uninstallKey),
    (Test-Path -LiteralPath $appKey),
    (Test-Path -LiteralPath $startMenuShortcut),
    (Test-Path -LiteralPath $desktopShortcut)
)
if ($preExisting -contains $true) {
    throw 'An existing OpenTermius installation, shortcuts or registry keys were found. This test only runs on a clean system so it cannot damage a real installation.'
}
if (Get-Process -Name 'opentermius' -ErrorAction SilentlyContinue) {
    throw 'OpenTermius is running. Close it before running the installer test.'
}

try {
    if (-not $configExisted) {
        [void][IO.Directory]::CreateDirectory($appDataDir)
    }
    [IO.File]::WriteAllText($sentinel, 'OpenTermius installer test data', [Text.UTF8Encoding]::new($false))
    $results.configKeptSentinel = $sentinel

    if (Test-Path -LiteralPath $installDir) { throw "Temporary install directory already exists: $installDir" }

    Invoke-Setup @('/S', "/D=$installDir") 'Silent install'

    $installedExe = Join-Path $installDir 'opentermius.exe'
    $installedUninstaller = Join-Path $installDir 'uninstall.exe'
    Assert-True (Test-Path -LiteralPath $installedExe -PathType Leaf) 'installed opentermius.exe is missing'
    Assert-True (Test-Path -LiteralPath $installedUninstaller -PathType Leaf) 'installed uninstall.exe is missing'
    $results.programFiles = (Get-ChildItem -LiteralPath $installDir -File | Select-Object -ExpandProperty Name) -join ', '

    $installedVersion = [Diagnostics.FileVersionInfo]::GetVersionInfo($installedExe)
    Assert-True ($installedVersion.ProductName -eq 'OpenTermius') 'installed executable has the wrong ProductName'
    Assert-True ($installedVersion.FileDescription -eq 'OpenTermius') 'installed executable has the wrong FileDescription'
    Assert-True ($installedVersion.CompanyName -eq 'OpenTermius') 'installed executable has the wrong CompanyName'
    Assert-True ($installedVersion.ProductVersion -eq $version) 'installed executable has the wrong ProductVersion'
    $installedHash = (Get-FileHash -LiteralPath $installedExe -Algorithm SHA256).Hash

    $shortcutShell = New-Object -ComObject WScript.Shell
    Assert-True (Test-Path -LiteralPath $startMenuShortcut -PathType Leaf) 'Start Menu shortcut is missing'
    $startShortcut = $shortcutShell.CreateShortcut($startMenuShortcut)
    Assert-True ($startShortcut.TargetPath -eq $installedExe) 'Start Menu shortcut points to the wrong target'
    Assert-True (Test-Path -LiteralPath $desktopShortcut -PathType Leaf) 'Desktop shortcut is missing'
    $desktopShortcutInfo = $shortcutShell.CreateShortcut($desktopShortcut)
    Assert-True ($desktopShortcutInfo.TargetPath -eq $installedExe) 'Desktop shortcut points to the wrong target'
    $results.shortcuts = 'Start Menu + Desktop'

    $entry = Get-ItemProperty -LiteralPath $uninstallKey -ErrorAction Stop
    Assert-True ($entry.DisplayName -eq 'OpenTermius') 'uninstall DisplayName is wrong'
    Assert-True ($entry.DisplayVersion -eq $version) "uninstall DisplayVersion is '$($entry.DisplayVersion)', expected '$version'"
    Assert-True ($entry.Publisher -eq 'OpenTermius') 'uninstall Publisher is wrong'
    Assert-True ($entry.DisplayIcon -like "$installedExe,*") 'uninstall DisplayIcon does not point at the installed executable'
    Assert-True ([int]$entry.EstimatedSize -gt 0) 'uninstall EstimatedSize is missing or zero'
    Assert-True ($entry.UninstallString -like "*$installedUninstaller*") 'uninstall UninstallString is wrong'
    Assert-True ($entry.QuietUninstallString -like "*$installedUninstaller* /S*") 'uninstall QuietUninstallString is wrong'
    $results.uninstallEntry = [ordered]@{
        DisplayName = $entry.DisplayName
        DisplayVersion = $entry.DisplayVersion
        Publisher = $entry.Publisher
        EstimatedSize = $entry.EstimatedSize
        UninstallString = $entry.UninstallString
        QuietUninstallString = $entry.QuietUninstallString
    }

    # Installing again exercises the overwrite/upgrade path without an uninstall step.
    Invoke-Setup @('/S', "/D=$installDir") 'Reinstall over existing version'
    Assert-True (Test-Path -LiteralPath $installedExe -PathType Leaf) 'opentermius.exe disappeared after reinstall'
    Assert-True (Test-Path -LiteralPath $installedUninstaller -PathType Leaf) 'uninstall.exe disappeared after reinstall'
    Assert-True ((Get-FileHash -LiteralPath $installedExe -Algorithm SHA256).Hash -eq $installedHash) 'reinstall changed the application payload'
    Assert-True ((Get-ItemProperty -LiteralPath $uninstallKey).InstallLocation -eq $installDir) 'reinstall changed the install directory'
    Assert-True (Test-Path -LiteralPath $sentinel -PathType Leaf) 'reinstall removed user configuration'
    $results.reinstall = 'ok'

    if ($KeepInstalled) {
        $results.status = 'installed (kept because -KeepInstalled was used)'
        $results | ConvertTo-Json -Depth 5
        return
    }

    # NSIS uninstallers relaunch themselves from a temporary directory, so wait
    # for the observable cleanup instead of trusting the process exit alone.
    $uninstallExit = Invoke-Uninstaller $installedUninstaller @('/S')
    $results.uninstallExitCode = $uninstallExit
    Wait-Condition { -not (Test-Path -LiteralPath $uninstallKey) } 'uninstall registry key removal'
    Wait-Condition { -not (Test-Path -LiteralPath $installedExe) } 'installed executable removal'
    Wait-Condition { -not (Test-Path -LiteralPath $installedUninstaller) } 'uninstaller removal'
    Wait-Condition { -not (Test-Path -LiteralPath $installDir) } 'install directory removal'

    Assert-True (-not (Test-Path -LiteralPath $startMenuShortcut)) 'Start Menu shortcut was not removed'
    Assert-True (-not (Test-Path -LiteralPath $desktopShortcut)) 'Desktop shortcut was not removed'
    Assert-True (-not (Test-Path -LiteralPath $appKey)) 'HKCU\Software\OpenTermius was not removed'
    Assert-True (Test-Path -LiteralPath $sentinel -PathType Leaf) 'user configuration was removed by a normal uninstall'

    $results.cleanup = 'install directory, shortcuts and registry entries removed'
    $results.configKept = $true
    $results.status = 'passed'
    $results | ConvertTo-Json -Depth 5
} finally {
    if (-not $KeepInstalled) {
        if ((Test-Path -LiteralPath $uninstallKey) -and (Test-Path -LiteralPath $installDir)) {
            $leftoverUninstaller = Join-Path $installDir 'uninstall.exe'
            if (Test-Path -LiteralPath $leftoverUninstaller) {
                [void](Invoke-Uninstaller $leftoverUninstaller @('/S'))
            }
            try { Wait-Condition { -not (Test-Path -LiteralPath $installDir) } 'test cleanup' 20 } catch { }
        }
        if (Test-Path -LiteralPath $installDir) {
            $tempRoot = [IO.Path]::GetFullPath([IO.Path]::GetTempPath()).TrimEnd('\') + '\'
            if ([IO.Path]::GetFullPath($installDir).StartsWith($tempRoot, [StringComparison]::OrdinalIgnoreCase)) {
                Remove-Item -LiteralPath $installDir -Recurse -Force -ErrorAction SilentlyContinue
            }
        }
        if (Test-Path -LiteralPath $startMenuShortcut) { Remove-Item -LiteralPath $startMenuShortcut -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $desktopShortcut) { Remove-Item -LiteralPath $desktopShortcut -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $uninstallKey) { Remove-Item -LiteralPath $uninstallKey -Recurse -Force -ErrorAction SilentlyContinue }
        if (Test-Path -LiteralPath $appKey) { Remove-Item -LiteralPath $appKey -Recurse -Force -ErrorAction SilentlyContinue }
    }
    if (Test-Path -LiteralPath $sentinel) { Remove-Item -LiteralPath $sentinel -Force -ErrorAction SilentlyContinue }
    if (-not $configExisted -and (Test-Path -LiteralPath $appDataDir -PathType Container)) {
        $remaining = @(Get-ChildItem -LiteralPath $appDataDir -Force)
        if ($remaining.Count -eq 0) { Remove-Item -LiteralPath $appDataDir -Force -ErrorAction SilentlyContinue }
    }
}
