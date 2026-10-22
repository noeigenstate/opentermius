# Windows 安装程序

OpenTermius 的发布产物是 NSIS 3 安装程序，不再提供便携 exe。安装程序脚本为 `installer.nsi`，由 `../scripts/package.ps1` 负责读取 `Cargo.toml` 版本、暂存 release 可执行文件并调用 `makensis.exe`。

默认构建输入为 `native/target/release/opentermius.exe`。显式设置 `CARGO_BUILD_TARGET` 时，输入改为 `target/<triple>/release/opentermius.exe`；也可用 `OPENTERMIUS_TEST_EXECUTABLE` 指定已验证的可执行文件。验证脚本读取 `native/releases/` 中的安装包，不依赖 Cargo 的目标目录布局。

## 构建

```powershell
./native/scripts/package.ps1
```

脚本依次从 `-MakensisPath`、`PATH`、`%ProgramFiles(x86)%\NSIS`、`%ProgramFiles%\NSIS`、`%LOCALAPPDATA%\electron-builder\Cache\nsis\nsis-*` 查找 `makensis.exe`。也可以直接构建，但必须自行准备 `staging/opentermius.exe` 并传入版本：

```powershell
makensis /DVERSION=0.3.3 installer.nsi
```

输出：`native/releases/OpenTermius-<version>-windows-x64-setup.exe`，SHA-256 写入 `native/releases/checksums.json`。

## 行为

- 默认按当前用户安装到 `%LOCALAPPDATA%\Programs\OpenTermius`，无需管理员权限。
- Modern UI 2，支持英文和简体中文；安装包与快捷方式使用 `assets/icon.ico`。
- 创建开始菜单快捷方式，桌面快捷方式可在组件页取消。
- 在 HKCU 卸载项中登记 DisplayName、DisplayVersion、Publisher、DisplayIcon、EstimatedSize、UninstallString 和 QuietUninstallString。
- 安装或卸载前检测 `opentermius.exe` 是否正在运行；交互模式询问用户关闭后重试，静默模式以非零退出码中止，避免覆盖正在使用的文件。
- 卸载删除程序文件、快捷方式和注册表项，默认保留 `%APPDATA%\OpenTermius`。卸载页可勾选“同时删除我的数据”，静默卸载可额外传入 `/REMOVEDATA`。

## 验证

```powershell
./native/installer/verify-install.ps1
```

脚本静默安装到独立临时目录，检查程序文件、快捷方式和卸载项，随后静默卸载并确认 `%APPDATA%\OpenTermius` 中的测试标记文件仍存在。若系统中已有 OpenTermius 安装、快捷方式、卸载项，或 OpenTermius 正在运行，脚本会拒绝运行，以免改动真实安装。可用 `-SetupPath` 指定安装包，或 `-KeepInstalled` 保留测试安装用于手动检查。
