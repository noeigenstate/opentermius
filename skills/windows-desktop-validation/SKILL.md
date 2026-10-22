---
name: windows-desktop-validation
description: Validate the OpenTermius Tauri 2 + React + Rust Windows desktop app, embedded frontend and existing NSIS installer.
---

# Windows desktop validation

Read `AGENTS.md` and `docs/tauri-architecture.md`. Run Cargo inside `native/`
to load its platform configuration. Run one local Cargo build at a time, use
Cargo on PATH, and wait for its machine gate/file locks. Do not change target
directories or run `cargo clean` to bypass locks. In this Windows workspace,
keep outputs, caches and temporary files on E:.

1. In `native/web/`, run `npm ci` then `npm run build` (Node 22). In `native/`,
   run `node scripts/check-versions.mjs`, `node --test scripts/check-versions.test.mjs`,
   `cargo build --locked --workspace` and `cargo test --locked --workspace`.
   Require zero Rust compiler warnings. Core SSH/SFTP, local PTY, credentials,
   persistence and recovery tests do not certify the WebView UI.
2. For development, run `npm run dev` in `native/web/`, then
   `cargo run --locked -p opentermius-desktop` in `native/`. Check touched pages
   in the real Tauri window: layout, translation, window controls, keyboard
   input and resizing. Terminal changes need SSH/local PTY input/output and
   visible-only size synchronization; SFTP changes need transfer, cancellation
   and confirmations. Use isolated test data; never save personal connection
   details or credentials in the repository.
3. From the repository root run `./native/scripts/package.ps1`. It builds the
   frontend and runs `cargo build --release --locked -p opentermius-desktop
   --features tauri/custom-protocol` inside `native/`, then uses our NSIS script.
   Expect `native/releases/OpenTermius-<version>-windows-x64-setup.exe` and
   `checksums.json`; check filename, version, bytes and SHA-256 agree.
   `-SkipBuild` is only for a release built with that exact feature and current
   frontend. `-MakensisPath` selects NSIS; `-DumpbinPath` records actual DLL
   imports. Do not infer CRT independence from flags. A cached bootstrapper
   passed via `-WebView2BootstrapperPath` must have a valid Microsoft signature.
4. Launch the final release with no Vite server and confirm its embedded UI
   loads. On a clean Windows account run `./native/installer/verify-install.ps1`
   to check silent install, overwrite upgrade, shortcuts, registration,
   uninstall and configuration retention. It refuses real installations and
   running apps; do not bypass that guard. `/D=<dir>` must be last and unquoted.
   Test missing WebView2 and failed runtime installation in a disposable VM;
   failure must exit nonzero before modifying app files. Do not remove the
   host machine's shared runtime to simulate this branch.

Keep GUI evidence under ignored `native/test-output/` and identify the platform
and executable tested. Old screenshots/tests are not Tauri evidence. The shell
must wire `--apply-update` before updater end-to-end verification. Windows
results do not certify macOS/Linux. See `native/installer/README.md` for details.
