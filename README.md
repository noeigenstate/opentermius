<p align="center"><img src="assets/logo.svg" width="112" alt="OpenTermius logo"></p>

<h1 align="center">OpenTermius</h1>

<p align="center">A lightweight, local-first SSH &amp; SFTP desktop client.<br>Tauri 2 · React 19 · TypeScript · Rust. No account required.</p>

## English

OpenTermius keeps connections and configuration on your device, without a product account, subscription, cloud sync or telemetry. The desktop app is being migrated from Slint to Tauri: React renders the interface in the system webview, while Rust owns SSH/SFTP, local processes, persistence and credentials. The built frontend is embedded in the shipping executable; Node.js and a development server are development tools only.

### Features and status

The Rust core provides saved hosts and groups, SSH password/private-key/interactive authentication, host fingerprint verification, local PTY terminals, SFTP transfers, key management, snippets, connection logs, encrypted recovery backups, Xshell/WindTerm import, server statistics and update checking. xterm.js is the terminal renderer for the new frontend. Desktop pages and commands are being ported incrementally; the initial Tauri shell exposes host listing. A successful build does not certify every migrated feature or desktop platform.

Credentials stay in Rust and are encrypted locally: Windows DPAPI, macOS Keychain-backed encryption, and a private master-key file on Linux. Exported hosts omit credentials and trust records. Linux's master-key file is not encrypted by an OS keyring; protect it and retain it with configuration backups. English and Simplified Chinese translations are bundled locally.

Application network access is limited to SSH/SFTP connections and the existing update service. The Windows installer additionally contacts Microsoft if WebView2 needs to be installed. The system webview/runtime has its own servicing policy.

### Install

Download `OpenTermius-<version>-windows-x64-setup.exe` and `checksums.json` from [Releases](https://github.com/noeigenstate/opentermius/releases). The Windows x64 installer is unsigned; SmartScreen may warn. Compare its SHA-256 with `checksums.json` before running it.

The existing NSIS installer installs per user, creates shortcuts and upgrades the existing directory in place. Microsoft Edge **WebView2 Runtime** is required: setup detects per-machine and per-user installations, and runs the bundled Microsoft-signed Evergreen bootstrapper if missing. That step needs internet access; for offline installation, preinstall the runtime using Microsoft's [Evergreen Standalone Installer](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#offline-deployment). Runtime installation failure stops setup before application files are replaced.

Uninstall keeps local configuration by default. The optional data-removal checkbox (or `/REMOVEDATA` for silent uninstall) removes it. macOS and Linux build coverage is configured in CI; signed releases and real desktop verification remain separate work.

### Development and tests

Prerequisites: Node.js 22, npm, stable Rust, and [platform Tauri dependencies](https://v2.tauri.app/start/prerequisites/). Windows needs Visual Studio C++ Build Tools and WebView2; macOS needs Xcode Command Line Tools. Linux packages (including WebKitGTK 4.1 and Ayatana AppIndicator) are listed in [CI](.github/workflows/ci.yml).

```sh
cd native/web
npm install
npm run dev
```

In a second terminal:

```sh
cd native
cargo run --locked -p opentermius-desktop
```

Alternatively, from `native/desktop/`, run `../web/node_modules/.bin/tauri dev` (PowerShell: `../web/node_modules/.bin/tauri.cmd dev`). This uses the web package's installed CLI and configured frontend development hook.

Run Cargo inside `native/` so its platform configuration is loaded. Keep one local Cargo build running at a time; do not change target directories to bypass file locks. In this Windows workspace, build output, caches and temporary files must remain on E:.

```sh
cd native
node scripts/check-versions.mjs
node --test scripts/check-versions.test.mjs
cargo build --locked --workspace
cargo test --locked --workspace
cd web
npm run build
```

See [Tauri architecture](docs/tauri-architecture.md) for layout and the command/event contract. Rust messages use `native/src/i18n_catalog.rs`; frontend Chinese translations live in `native/web/src/i18n/zh/`. Read `AGENTS.md` and matching repository skills before changes. Historical Slint screenshots and validation documents describe the previous UI and do not validate the new app.

### Build and release

From the repository root on Windows, with NSIS 3 installed:

```powershell
./native/scripts/package.ps1
```

The script runs `npm ci` and `npm run build` in `native/web/`, then this command in `native/`:

```sh
cargo build --release --locked -p opentermius-desktop --features tauri/custom-protocol
```

`tauri/custom-protocol` embeds `web/dist` into the Tauri executable. A plain Cargo release build does not enable the shipping asset protocol. Our existing `installer/installer.nsi` packages `target/release/opentermius.exe` (honoring Cargo's configured target directory), producing `native/releases/OpenTermius-<version>-windows-x64-setup.exe` and `checksums.json`. Tauri's own bundler stays disabled. See [packaging details](native/installer/README.md) for NSIS discovery, cached bootstrapper input and `-SkipBuild`.

The canonical version is `native/desktop/tauri.conf.json`; it must match both Cargo manifests and the web manifest/lockfile (currently `0.5.0`). Packaging and both CI workflows enforce this. A matching `vX.Y.Z` tag triggers the release workflow, publishing the same installer asset name and checksum format consumed by the existing updater. Silent upgrades retain `installer /S /D=<install-dir>` with `/D` last and unquoted. The Tauri shell must also wire the core updater's `--apply-update` helper entry point before end-to-end in-app updates can be certified.

`native/installer/verify-install.ps1` tests silent install, overwrite upgrade, shortcuts, registration, uninstall and configuration retention on a clean Windows account. It refuses to run over a real installation. Host-side archive scripts for macOS/Linux also build with embedded assets; those packages are unsigned and require platform-specific validation.

## 中文

OpenTermius 是轻量、本地优先的 SSH/SFTP 桌面客户端，无需产品账号、订阅或云同步，也不收集遥测。项目正在从 Slint 迁移到 **Tauri 2 + React 19 + TypeScript + Rust**：React 在系统 WebView 中显示界面，Rust 负责连接、进程、配置和凭据。发布程序内嵌编译后的前端，运行时无需 Node.js 或开发服务器。

### 功能与状态

Rust 核心已包含主机与分组、SSH 密码/私钥/交互式认证、指纹校验、本地 PTY、SFTP、密钥管理、代码片段、连接日志、加密备份恢复、Xshell/WindTerm 导入、服务器状态和更新检查。新终端使用 xterm.js；各页面和命令正在逐步移植，初始 Tauri 外壳提供主机列表。构建成功不代表所有迁移功能或桌面平台已完成验收。

凭据由 Rust 管理并在本地加密：Windows 使用 DPAPI，macOS 使用 Keychain 主密钥，Linux 使用私有主密钥文件。主机导出不包含凭据或信任记录。Linux 主密钥文件未由系统钥匙串加密，应妥善保护并随配置备份。英文和简体中文翻译随程序离线提供。

应用联网范围为 SSH/SFTP 与已有更新服务；Windows 安装程序仅在缺少 WebView2 时联系 Microsoft 安装运行时。系统 WebView/运行时有独立的维护更新策略。

### 安装

从 [Releases](https://github.com/noeigenstate/opentermius/releases) 下载 `OpenTermius-<version>-windows-x64-setup.exe` 与 `checksums.json`，核对 SHA-256 后安装。当前安装包未签名，SmartScreen 可能提示警告。

沿用 NSIS 按当前用户安装，创建快捷方式并覆盖升级原目录。程序需要 **WebView2 Runtime**：安装程序检查机器级和用户级注册信息，缺失时运行随包携带、经 Microsoft 签名的 Evergreen 引导安装程序，此步骤需要联网。离线环境应预先安装 Microsoft 的 [Evergreen 完整离线安装包](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution#offline-deployment)。运行时安装失败会在替换程序文件前终止安装。

卸载默认保留本地配置；可勾选删除数据，或静默卸载时传入 `/REMOVEDATA`。macOS/Linux 已配置 CI 构建与测试；签名发行和真实桌面验收需分别完成。

### 开发与测试

需要 Node.js 22、npm、稳定版 Rust 及 Tauri 平台构建依赖。Windows 需要 Visual Studio C++ Build Tools 和 WebView2，macOS 需要 Xcode Command Line Tools，Linux 依赖见 [CI](.github/workflows/ci.yml)。

在 `native/web/` 执行 `npm install`，然后启动 `npm run dev`；另开终端，在 `native/` 执行 `cargo run --locked -p opentermius-desktop`。也可在 `native/desktop/` 使用 `../web/node_modules/.bin/tauri dev`（PowerShell 使用 `tauri.cmd`）。

```sh
cd native
node scripts/check-versions.mjs
node --test scripts/check-versions.test.mjs
cargo build --locked --workspace
cargo test --locked --workspace
cd web
npm run build
```

Cargo 应从 `native/` 内运行，每次仅启动一个本地 Cargo 构建，不通过切换目标目录绕过文件锁。本 Windows 工作区的构建产物、缓存与临时文件保留在 E:。接口约定见 [Tauri 架构文档](docs/tauri-architecture.md)，修改前阅读 `AGENTS.md` 及相关技能。历史 Slint 截图与验证记录不代表新界面已验收。

### 构建与发布

安装 NSIS 3 后，在仓库根目录执行 `./native/scripts/package.ps1`。脚本先执行前端 `npm ci` 和 `npm run build`，再构建 `opentermius-desktop`，显式启用 `tauri/custom-protocol` 内嵌静态资源。沿用现有 NSIS 安装脚本，输出 `native/releases/OpenTermius-<version>-windows-x64-setup.exe` 与 `checksums.json`，不使用 Tauri 自带安装器。详细参数见 [安装程序说明](native/installer/README.md)。

版本以 `native/desktop/tauri.conf.json` 为准，必须与两个 Cargo 清单及前端清单/锁文件一致，当前为 `0.5.0`。脚本和 CI 会检查一致性。匹配的 `vX.Y.Z` 标签触发发布，保持已有更新器使用的文件名、校验格式及 `/S /D=<安装目录>` 升级协议（`/D` 必须最后且不加引号）。Tauri 外壳还需接入核心更新器的 `--apply-update` 入口，才能完成应用内更新端到端验收。

在干净 Windows 账号运行 `native/installer/verify-install.ps1` 可验证静默安装、覆盖升级、快捷方式、注册项、卸载和配置保留；脚本发现真实安装时会拒绝运行。macOS/Linux 归档脚本也会内嵌前端，但产物未签名，仍需对应平台验证。
