# 跨平台验证记录（2026-10-20，合并 main 后复验）

宿主为 Windows x64，Rust 1.97.1 MSVC、Slint 1.18.1。已将本地 main（37d6653）合并到 feat/crossplatform。所有 Cargo 命令均从 native/ 执行，使用 PATH 上的机器共享队列；没有设置单独的 CARGO_TARGET_DIR，没有推送或触发远端 CI。

| 检查 | 本次结果与边界 |
| --- | --- |
| cargo build --locked | 最终源码通过，退出码 0，无警告；默认产物为 target/debug/opentermius.exe |
| cargo test --locked | 最终源码通过，249 项通过、0 项失败、4 项交互式测试按设计忽略；无警告 |
| cargo build --release --locked | 最终源码通过，退出码 0，无警告；产物为 target/release/opentermius.exe |
| release 原生 GUI 集成测试 | 设置 OPENTERMIUS_TEST_EXECUTABLE 为最终 release 后执行 cargo test --locked -- --ignored --nocapture，4 项均通过，无警告；涵盖密码/Keychain 转发自动启动和退出清理、SFTP、SSH 认证/指纹、DPAPI 及配置持久化 |
| 调试版 GUI 截图 | 已运行并查看 test-output/integration-recovery/native-window.png 及 test-output/integration-terminal/native-window.png：Backup & Restore 弹窗、Unicode 终端及重连提示正常，无裁切 |
| Linux x86_64-unknown-linux-gnu | 最终源码 cargo check --locked --target x86_64-unknown-linux-gnu --all-targets --config target/port-cross-check.toml 通过，无警告；使用 Zig 及下述 fontconfig 元数据覆盖 |
| macOS aarch64-apple-darwin | 最终源码 cargo check --locked --target aarch64-apple-darwin --all-targets 通过，无警告；使用 Zig，仅验证类型和元数据 |
| Unix 打包脚本 | 两个脚本均通过 ShellCheck 0.11.0；本次未在 Windows 上执行 Unix 打包 |
| Windows 打包及安装验证 | package.ps1 -SkipBuild 从默认 target/release 路径生成最终 NSIS 安装程序及 SHA-256；installer/verify-install.ps1 通过：隔离目录静默安装、覆盖安装、快捷方式和 HKCU 卸载项检查、卸载及清理、保留用户配置 |
| PowerShell 与 Slint 静态检查 | package.ps1、installer/verify-install.ps1 语法解析通过；main.slint 为每文件一个 import、每行一个 export；git diff --check 通过 |

跨目标检查涵盖库、完整应用和目标平台启用的测试代码，包括 keychain、恢复码、Xshell 导入、主题设置、SFTP 和终端。首轮 Linux 检查发现 keychain 测试模块的 Windows 专用通配导入在 Unix 上未使用；已仅对该导入增加 cfg(windows)，保留平台无关测试，复验无警告。

另已查看最终 release 的 test-output/integration-release-live/native-window.png 和 test-output/integration-release-sftp/native-window.png：终端 Unicode、光标、远程文件侧栏及双栏 SFTP 显示正常。SFTP 集成测试现在和其他 GUI 测试一样支持 OPENTERMIUS_TEST_EXECUTABLE，未设置时使用 Cargo 提供的默认二进制路径。

交叉 C 编译沿用 Zig 0.16.0。仅在命令进程内设置目标专用 CC/AR、CRATE_CC_NO_DEFAULTS=1 及 native/target/ 内的 Zig 缓存；命令包装器位于忽略的 target/integration-tools/，分别调用 zig cc -target x86_64-linux-gnu、zig cc -target aarch64-macos 和 zig ar。构建输出使用默认 target/ 目录。

Windows 宿主没有 Linux fontconfig sysroot/pkg-config 交叉配置，因此 Linux 检查沿用忽略的 target/port-cross-check.toml：

```toml
[target.x86_64-unknown-linux-gnu.fontconfig]
rustc-link-lib = ["fontconfig"]
```

此覆盖跳过 yeslogic-fontconfig-sys 的系统库探测，仅声明链接元数据，保留 Rust FFI 类型检查。它不证明系统库存在或可以链接，也未进入项目默认配置或 CI。macOS 检查同样没有执行原生链接或启动应用。

Windows DPAPI 原格式保持不变；keychain 和恢复码继续统一使用 store::protect/unprotect。Unix 使用原有便携加密后端。注册表访问仅在 Windows 编译；Unix 的 Xshell 默认目录为 None、System 主题回退浅色，打开配置目录在 macOS/Linux 分别使用 open/xdg-open。终端测量、显示和输入法共享平台字体，并保留 main 的字号及终端行为。

尚未在 macOS/Linux 真机运行 GUI、Keychain 授权、Unix PTY 和 Unix 专属测试，也未执行 Unix 打包或原生 release 链接；这些仍需对应宿主及 CI 验证。Linux 主密钥仍为用户拥有的 0600 文件，未接入 Secret Service，失去该文件会失去已保存凭据。CI 已配置但本次没有远端运行结果，发布产物未签名/公证。
