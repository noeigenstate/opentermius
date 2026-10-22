# Windows installer

Run `./native/scripts/package.ps1` from the repository root with Node 22, Rust
MSVC, Visual Studio C++ Build Tools and NSIS 3 installed. It runs `npm ci` and
`npm run build` in `native/web/`, then builds inside `native/`:

```sh
cargo build --release --locked -p opentermius-desktop --features tauri/custom-protocol
```

`tauri/custom-protocol` embeds `web/dist` into the executable. The default input
is `native/target/release/opentermius.exe`; Cargo metadata resolves configured
target directories and `CARGO_BUILD_TARGET` selects the triple subdirectory.
Only Windows x64 MSVC is supported by this installer. Tauri bundling stays
disabled: `installer.nsi` retains the existing upgrade identity and behavior.

The version comes from `desktop/tauri.conf.json`. `scripts/check-versions.mjs`
requires matching versions in both Cargo manifests and the web manifest and
lockfile. The packager also checks the executable's Tauri product metadata and
version to reject stale releases.

Outputs remain:

- `native/releases/OpenTermius-<version>-windows-x64-setup.exe`
- `native/releases/checksums.json` (version, product, architecture, installer,
  filename, bytes and SHA-256; existing updater format)

NSIS lookup order is `-MakensisPath`, `PATH`, standard Program Files directories,
then the local electron-builder NSIS cache. If needed, install NSIS with:

```powershell
winget install -e --id NSIS.NSIS --accept-source-agreements --accept-package-agreements
```

`-SkipBuild` skips frontend and Rust compilation for a previously verified release
built with the exact command above; the caller must ensure it contains the
current frontend. `OPENTERMIUS_TEST_EXECUTABLE` is accepted only with `-SkipBuild`.
`-DumpbinPath` records actual direct DLL imports in the checksum manifest.

## WebView2

The packager downloads the small [Microsoft Evergreen bootstrapper](https://go.microsoft.com/fwlink/p/?LinkId=2124703),
checks its Authenticode signature is valid and issued to Microsoft Corporation,
and embeds it in NSIS. `-WebView2BootstrapperPath` supplies a cached copy for an
offline packaging machine; it undergoes the same signature check.

Following [Microsoft's runtime distribution guidance](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution),
setup checks `pv` for `{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}` in the 32-bit
HKLM `Software\Microsoft\EdgeUpdate\Clients` view and the HKCU location. Empty
values and `0.0.0.0` mean missing. It runs the bundled bootstrapper with
`/silent /install` only if neither registration is valid, and rechecks registration
after installation. Failure exits with code 4 before replacing application files,
including in silent mode. The bootstrapper needs internet to fetch the runtime;
offline target machines must preinstall Microsoft's Evergreen Standalone Runtime.
WebView2 is shared and is retained when OpenTermius is uninstalled.

## Compatibility and validation

NSIS keeps the same per-user install directory, HKCU application/uninstall keys,
English/Chinese UI and shortcut options. Upgrade uses `installer /S /D=<dir>`:
`/D` must be last and unquoted, even with spaces. A running OpenTermius causes
silent setup to exit with code 2. Normal uninstall retains configuration;
`/REMOVEDATA` explicitly removes it. The existing updater's checksum schema and
asset names are unchanged. The Tauri shell still needs its `--apply-update`
argument dispatch before in-app update/restart can be certified end to end.

On a clean Windows account, run:

```powershell
./native/installer/verify-install.ps1
```

It tests silent install, overwrite upgrade, program metadata, shortcuts, HKCU
registration, uninstall and configuration retention. It refuses existing
installations/shortcuts/registry keys or running instances. `-SetupPath` selects
an installer; `-KeepInstalled` retains the isolated test installation. Never
bypass these guards on a real installation. Validate missing-runtime and
network-failure branches in a disposable VM rather than removing a shared runtime.
Also launch the final executable with no Vite server to check embedded assets.
