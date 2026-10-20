<p align="center"><img src="assets/logo.svg" width="112" alt="OpenTermius logo"></p>

<h1 align="center">OpenTermius</h1>

<p align="center">A lightweight, local-first SSH &amp; SFTP client.<br>Built with Rust and Slint. No account required.</p>

OpenTermius 面向 **Windows、macOS 和 Linux** 打造轻量原生终端体验，将主机管理、远程会话和文件传输放在同一个工作区。连接信息保存在本机，无需注册产品账号，没有云同步、订阅或遥测服务。

| 平台 | 当前状态 |
| --- | --- |
| Windows x64 | 可运行的预览版本，已验证 |
| macOS | 计划支持 |
| Linux | 计划支持 |

![OpenTermius](docs/screenshots/home.png)

## 当前功能

- 主机创建与编辑、搜索、分组筛选和 JSON 导入/导出。
- SSH 密码、私钥及交互式认证；首次连接确认 SHA-256 指纹，指纹变化时拒绝连接。
- 多标签终端、Unicode/ANSI 输出、窗口尺寸同步、历史滚动和复制粘贴。
- 本地 PowerShell 终端，使用 Windows ConPTY 管理进程。
- SFTP 双栏文件浏览、流式上传与下载；上传覆盖需确认，下载不覆盖已有文件。
- Windows DPAPI 加密保存密码和密钥口令；导出文件不含凭据或主机信任记录。

## 安装

Windows 安装程序：`native/releases/OpenTermius-0.2.0-windows-x64-setup.exe`。双击安装，默认按当前用户安装到 `%LOCALAPPDATA%\Programs\OpenTermius`，无需管理员权限。安装程序提供开始菜单快捷方式，并可选择创建桌面快捷方式；安装或覆盖升级前会检查 OpenTermius 是否正在运行。

卸载会删除程序文件和快捷方式，但默认保留 `%APPDATA%\OpenTermius` 中的配置。如需一并删除主机、凭据和偏好设置，请在卸载界面勾选“同时删除我的数据”。

配置保存在系统配置目录的 `OpenTermius/opentermius-native.json`。升级时可自动读取旧版本的本地配置，不覆盖已有的新配置。程序目前未配置发布者签名，构建产物不提交到源码仓库。

## 开发

当前构建与验证环境为 Windows x64、Rust 1.97.1 MSVC、Slint 1.18.1，需要 Visual Studio C++ Build Tools。首次获取依赖需要网络。

```powershell
cd native
cargo run --locked
cargo test --locked
cargo build --release --locked
```

请在 `native/` 内执行 Cargo，以加载该目录的目标配置。当前默认构建目标为 Windows MSVC；macOS 和 Linux 的打包、系统凭据及平台适配尚未完成。

在仓库根目录打包 Windows 安装程序及 SHA-256 清单：

```powershell
./native/scripts/package.ps1
```

产物为 `native/releases/OpenTermius-<version>-windows-x64-setup.exe` 和 `native/releases/checksums.json`，不再生成便携 exe。打包需要 NSIS 3；脚本会从 `-MakensisPath`、`PATH`、标准安装目录或 `%LOCALAPPDATA%\electron-builder\Cache\nsis` 查找 `makensis.exe`。未安装时可执行：

```powershell
winget install -e --id NSIS.NSIS --accept-source-agreements --accept-package-agreements
```

安装程序验证会将安装目录设为独立临时目录，静默安装、检查快捷方式和 HKCU 卸载项，再静默卸载并确认用户配置保留：

```powershell
./native/installer/verify-install.ps1
```

实际 GUI 集成测试需交互式 Windows 桌面，使用临时 SSH/SFTP 服务及隔离配置：

```powershell
cd native
$env:OPENTERMIUS_TEST_EXECUTABLE = (Resolve-Path target/x86_64-pc-windows-msvc/release/opentermius.exe).Path
$env:OPENTERMIUS_SMOKE_OUTPUT = Join-Path (Get-Location) 'test-output/release-live'
cargo test --test transport native_gui_live_selftest -- --ignored --nocapture
```

## 自动构建

[Windows installer 工作流](.github/workflows/windows-release.yml) 在推送到 `main`、向 `main` 提交 PR、推送 `v*` 标签时运行，也可在 GitHub Actions 中通过 **Run workflow** 手动触发。工作流使用 Windows runner 和 Rust stable，依次运行锁定依赖的测试、release 构建及 NSIS 打包；需要交互式桌面的 GUI 测试保持忽略。

构建成功后，在该次运行的 **Artifacts** 中下载 `OpenTermius-windows-x64-setup`，内含 `OpenTermius-<version>-windows-x64-setup.exe` 和 `checksums.json`。推送与 `native/Cargo.toml` 版本一致的 `vX.Y.Z` 标签还会自动创建 GitHub Release，附上这两个文件并在发布说明中列出 SHA-256；版本不一致会直接失败。安装程序尚未签名，Windows SmartScreen 可能提示警告，可使用 `Get-FileHash <安装程序路径> -Algorithm SHA256` 核对清单中的哈希值。

## 设计与实现

界面由 Slint 直接渲染，使用系统字体和项目绘制的 SVG 图标。应用标识源文件为 [assets/logo.svg](assets/logo.svg)，Windows 程序内嵌多尺寸图标及 OpenTermius 版本信息。修改标识后，可使用 ImageMagick 执行 `./scripts/generate-icons.ps1` 重新生成 PNG/ICO；正常构建无需 ImageMagick。

SSH/SFTP 使用 russh 与 russh-sftp，终端屏幕使用 vt100，异步任务由 Tokio 调度。应用不需要 Electron、Chromium、WebView 或 Node.js 运行时。

实现细节见 [架构文档](docs/architecture.md)，验证结果见 [validation.json](docs/validation.json)。

## 开发状态

当前是预览版本。密钥管理、端口转发、代码片段、已知主机管理、日志及完整偏好设置仍在计划中。尚未提供 Mosh、串口、Telnet、FIDO2 或证书认证。终端尚未覆盖全部 xterm 行为、输入法及长期压力场景。
