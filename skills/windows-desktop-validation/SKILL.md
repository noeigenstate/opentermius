---
name: windows-desktop-validation
description: 修改或打包本项目 Rust/Slint Windows 客户端时，验证原生 UI、SSH/SFTP、ConPTY、凭据及发布产物。
---

源码位于 `native/`。在该目录执行 Cargo，使 `.cargo/config.toml` 的 Windows x64 MSVC 静态 CRT 配置生效；默认目标为宿主平台。仅在仓库根目录传入 manifest 路径不会加载该目录配置。

## 功能与产物验证

1. 在 `native/` 执行 `cargo test --locked`。传输测试启动临时回环 SSH/SFTP 服务；本地终端测试在 Windows 启动真实 PowerShell，在 Unix 启动用户登录 shell，并检查断开、退出和任务取消后的子进程回收。测试以退出码为准。macOS/Linux 另需对应宿主的构建与桌面证据。
2. 在交互式 Windows 桌面执行 `cargo test --test transport native_gui_live_selftest -- --ignored --nocapture`。测试通过真实原生窗口回调检查认证弹窗、服务器指纹、SSH 回显、SFTP、DPAPI、持久化与截图，使用独立临时配置。
3. 执行 `cargo build --release --locked`。默认产物为 `target/release/opentermius.exe`（Unix 为 `opentermius`）；显式 `--target <triple>` 时为 `target/<triple>/release/...`，也须考虑 `CARGO_TARGET_DIR`。将 `OPENTERMIUS_TEST_EXECUTABLE` 设为最终 release 可执行文件的绝对路径，再运行上述 GUI 测试，验证实际发布产物。测试默认使用 Cargo 提供的二进制路径，不应硬编码 Windows triple。`OPENTERMIUS_SMOKE_OUTPUT` 可指定截图与 JSON 的输出目录。
4. 仓库根目录运行 `./native/scripts/package.ps1 -SkipBuild` 打包已验证的版本，生成 NSIS 安装程序 `native/releases/OpenTermius-<version>-windows-x64-setup.exe` 及 `checksums.json`；不再生成便携 exe。脚本按 `-MakensisPath`、`PATH`、标准安装目录、`%LOCALAPPDATA%\electron-builder\Cache\nsis` 的顺序查找 `makensis.exe`，找不到时按提示使用 `winget install -e --id NSIS.NSIS --accept-source-agreements --accept-package-agreements` 安装。可传入 `-DumpbinPath <dumpbin>` 检查最终程序的 DLL 导入，不能仅凭构建配置断言不依赖 VC 运行库。
5. 运行 `./native/installer/verify-install.ps1` 验证安装程序：以 `/S /D=<独立临时目录>` 静默安装（`/D` 必须是最后一个参数且不加引号），检查程序文件、开始菜单及桌面快捷方式、HKCU 卸载项中的版本与路径，再静默卸载并确认安装目录、快捷方式和注册表项清理干净、`%APPDATA%\OpenTermius` 配置保留。测试结束后不得留下注册表项或快捷方式；测试脚本发现已有安装或 OpenTermius 正在运行时会拒绝运行，避免破坏真实环境。

## 界面对照

使用最终程序的 `--smoke-test --smoke-scene=empty|editor|hosts|sftp --data-dir=<fresh-temp> --output=<output>` 生成对应原生截图；数据目录必须独立。检查布局、裁切、系统字体、空状态和真实会话状态。计划中的页面和平台应明确标示，不能把 Windows 验证结果扩展为其他平台已可用。

应用标识维护在 `assets/logo.svg`，通过 `scripts/generate-icons.ps1` 生成 PNG 和多尺寸 ICO。修改后验证 Slint 窗口图标、Windows EXE 内嵌图标及版本资源中的产品名；只改窗口标题不能改变资源管理器里的图标和产品信息。正常构建使用已提交的图标文件，无需 ImageMagick。

Slint 绝对定位组件要设置实际 `width/height`；仅设置最小或最大值可能使按钮铺满父容器。终端单元格宽度通过实际 Consolas 文本测量，不用猜测常量；输出、光标、选择和列数应共用测量值。

## 资源与脚本

`scripts/measure-process.ps1` 对新临时配置进行进程树采样，清理时核对 PID 与创建时间。区分工作集、私有字节、采样时长、渲染模式和启动就绪条件。截图模式的固定等待计时不能当作启动性能；短时空闲 CPU 为零不能证明长期稳定性。

PowerShell 5.1 读写源码显式使用 UTF-8，编辑优先使用 apply_patch。默认 `$OutputEncoding` 可能将管道送入 Python 的中文转换为问号；不要用默认 here-string 管道替换中文源码。递归清理前验证解析后的绝对目标仍在预期目录内，使用同一运行时完成检查与删除。
