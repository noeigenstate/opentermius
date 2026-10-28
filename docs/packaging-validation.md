# Tauri packaging validation

Local scope: Windows x64, 2026-10-22, version 0.5.0. These results apply to
packaging and the initial desktop shell, not all migrated desktop features.

| Check | Result |
| --- | --- |
| `cargo build --locked --workspace` in `native/` | Passed, zero compiler warnings |
| `cargo test --locked --workspace` in `native/` | 104 unit and 168 integration tests passed; 2 existing live tests ignored; zero compiler warnings |
| `npm ci`, `npm run build` in `native/web/` | Passed; local npm configuration emitted tooling warnings |
| Version check and its Node tests | 0.5.0 agrees across both Cargo manifests, Tauri config and npm manifest/lockfile; 2 tests passed |
| Full `native/scripts/package.ps1` | Passed: frontend rebuilt, custom-protocol release compiled with zero warnings, Microsoft-signed bootstrapper downloaded and NSIS installer generated |
| `package.ps1 -SkipBuild -WebView2BootstrapperPath <cached-bootstrapper>` | Passed, including x64 PE and product/version checks |
| Installer checksum manifest | Filename, byte length and SHA-256 match the generated installer |
| Release startup | Real WebView accessibility exposed the React interface at `http://tauri.localhost/`; the owned release process exited cleanly |
| PowerShell/Bash parsing, workflow YAML, `git diff --check` | Passed |

The release startup used its embedded asset origin while another worker's
development server remained untouched. Window capture failed with
`window capture timed out: timed out waiting on channel`; screenshot-based visual
inspection remains unverified. The attempted closed-proxy WebView option was not
observed in the browser command line and is not counted as an offline test.

`installer/verify-install.ps1` refused the existing local installation as intended;
it did not modify it. Clean silent install, overwrite upgrade and uninstall are
configured in `release.yml` and still need a clean-account/CI run. The
missing-WebView2 and failed-bootstrapper branches require a disposable Windows
VM. No runtime was removed from this machine. macOS/Linux CI and archive scripts
were not executed locally. The initial Tauri shell also needs `--apply-update`
dispatch before an end-to-end in-app updater test can pass.

Generated installers, checksum manifests, browser data and other build artifacts
are not committed. Run packaging again to obtain a fresh manifest; NSIS output
can differ between runs even when the application payload is unchanged.
