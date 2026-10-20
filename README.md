<p align="center"><img src="assets/logo.svg" width="112" alt="OpenTermius logo"></p>

<h1 align="center">OpenTermius</h1>

<p align="center">A lightweight, local-first SSH &amp; SFTP client.<br>Built with Rust and Slint. No account required.</p>

OpenTermius 面向 **Windows、macOS 和 Linux** 打造轻量原生终端体验，将主机管理、远程会话和文件传输放在同一个工作区。连接信息保存在本机，无需注册产品账号，没有云同步、订阅或遥测服务。

| 平台 | 当前状态 |
| --- | --- |
| Windows x64 | 可运行的预览版本，已验证 |
| macOS | aarch64 交叉检查已通过；已配置 CI，待首次通过；尚未经真机手工验证 |
| Linux | Rust 全目标交叉类型检查已通过（见验证记录）；原生 CI 和真机验证待完成 |

![OpenTermius](docs/screenshots/home.png)

## 当前功能

- 主机创建与编辑、搜索、分组筛选和 JSON 导入/导出。
- SSH 密码、私钥及交互式认证；首次连接确认 SHA-256 指纹，指纹变化时拒绝连接。
- 多标签终端、Unicode/ANSI 输出、窗口尺寸同步、历史滚动和复制粘贴。
- 本地终端：Windows 使用 ConPTY/PowerShell；macOS/Linux 使用 PTY 和用户的 `$SHELL` 登录 shell。
- SFTP 双栏文件浏览、流式上传与下载；上传覆盖需确认，下载不覆盖已有文件。
- 加密保存密码和密钥口令：Windows DPAPI；macOS Keychain 主密钥 + AES-256-GCM；Linux 本地主密钥 + AES-256-GCM。主机导出不含凭据或信任记录。

## 启动

Windows 便携程序：`native/releases/OpenTermius-0.2.0-windows-x64.exe`，双击运行。构建产物不提交到源码仓库。

配置保存在系统配置目录的 `OpenTermius/opentermius-native.json`。升级时可自动读取旧版本的本地配置，不覆盖已有的新配置。程序目前未配置发布者签名。

## 开发

本机验证环境为 Windows x64、Rust 1.97.1 MSVC、Slint 1.18.1，需要 Visual Studio C++ Build Tools。macOS 需要 Xcode Command Line Tools；Linux 的系统开发依赖见 `.github/workflows/ci.yml`。首次获取依赖需要网络，应用运行时不依赖网络服务（SSH/SFTP 连接除外）。

```powershell
cd native
cargo run --locked
cargo test --locked
cargo build --release --locked
```

请在 `native/` 内执行 Cargo，以加载该目录配置。默认使用宿主目标；Windows x64 MSVC 继续使用静态 CRT。默认产物位于 `target/debug` / `target/release`（Windows 带 `.exe`）；显式 `--target` 构建则位于 `target/<triple>/...`，`CARGO_TARGET_DIR` 可覆盖根目录。

Linux 需安装 X11/Wayland、fontconfig、xkbcommon 和等宽字体；文件对话框需要 XDG desktop portal 及桌面环境对应的 portal backend。终端字体为 Windows Consolas、macOS Menlo、Linux DejaVu Sans Mono，度量与渲染共享同一字体。

macOS 将随机 256 位主密钥保存为默认 Keychain 的 generic password，钥匙串锁定或拒绝访问时保存凭据会失败。Linux 为避免引入额外 Secret Service 依赖，使用配置目录下当前用户拥有、权限为 `0600` 的 `master-key-v1` 文件；配置目录不可由其他用户写入。**Linux 主密钥文件未由系统钥匙串加密**，无法防御已能读取当前用户文件的攻击者。备份配置时应安全备份该文件；丢失主密钥后无法恢复已保存的密码。非 Windows 凭据带 `v1:` 前缀，不支持直接解密其他机器/平台的已有凭据；Windows 原始 DPAPI 格式保持兼容。`--data-dir` 同时隔离便携加密主密钥。

在仓库根目录打包 Windows 便携程序及 SHA-256 清单：

```powershell
./native/scripts/package.ps1
```

在对应宿主系统运行 `bash native/scripts/package-macos.sh` 或 `bash native/scripts/package-linux.sh`，输出到 `native/releases/`。macOS 脚本使用 `sips` + `iconutil` 将已提交的 PNG 转换为 ICNS，生成 `.app` 和 `hdiutil` DMG；缺少图标工具时使用系统默认图标，未签名/公证。Linux 脚本生成含二进制、`.desktop`、PNG 图标及安装说明的 tar.gz，不捆绑系统动态库。两个脚本支持 `CARGO_BUILD_TARGET`、`CARGO_TARGET_DIR` 和 `OPENTERMIUS_TEST_EXECUTABLE`。尚未在对应系统执行打包脚本。

实际 GUI 集成测试需交互式 Windows 桌面，使用临时 SSH/SFTP 服务及隔离配置：

```powershell
cd native
$env:OPENTERMIUS_TEST_EXECUTABLE = (Resolve-Path target/release/opentermius.exe).Path
$env:OPENTERMIUS_SMOKE_OUTPUT = Join-Path (Get-Location) 'test-output/release-live'
cargo test --test transport native_gui_live_selftest -- --ignored --nocapture
```

## 设计与实现

界面由 Slint 直接渲染，使用系统字体和项目绘制的 SVG 图标。应用标识源文件为 [assets/logo.svg](assets/logo.svg)，Windows 程序内嵌多尺寸图标及 OpenTermius 版本信息。修改标识后，可使用 ImageMagick 执行 `./scripts/generate-icons.ps1` 重新生成 PNG/ICO；正常构建无需 ImageMagick。

SSH/SFTP 使用 russh 与 russh-sftp，终端屏幕使用 vt100，异步任务由 Tokio 调度。应用不需要 Electron、Chromium、WebView 或 Node.js 运行时。

实现细节见 [架构文档](docs/architecture.md)，本次适配验证见 [跨平台验证记录](docs/cross-platform-validation.md)，历史 Windows 发布产物验证见 [validation.json](docs/validation.json)。

## 开发状态

当前是预览版本。密钥管理、端口转发、代码片段、已知主机管理、日志及完整偏好设置仍在计划中。尚未提供 Mosh、串口、Telnet、FIDO2 或证书认证。终端尚未覆盖全部 xterm 行为、输入法及长期压力场景。
