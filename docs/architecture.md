# OpenTermius 架构

OpenTermius 使用 Rust、Slint 和 Tokio，采用 winit 窗口后端和软件渲染，发布配置启用 thin LTO、符号剥离。默认构建目标为宿主系统，Windows x64 MSVC 保留静态 CRT。macOS/Linux 已配置 CI 构建与测试，待 CI 首次通过，尚未经真机手工验证。

| 模块 | 职责 |
| --- | --- |
| `native/ui/main.slint` | 主机、编辑器、终端、认证提示和双栏文件界面 |
| `native/src/main.rs` | GUI 回调、会话状态、事件聚合、剪贴板及文件选择 |
| `native/src/store.rs`、`native/src/store/secrets.rs` 及子模块 | 本地配置、主机指纹、DPAPI/便携加密与配置版本迁移 |
| `native/src/ssh.rs` | SSH 认证、通道、SFTP、文件传输与取消 |
| `native/src/local.rs` | Windows ConPTY/PowerShell、Unix PTY/登录 shell 生命周期 |
| `native/src/terminal.rs` | vt100 屏幕、历史、颜色、查询回复及文本选择 |
| `assets/` | 应用标识和生成的 PNG/ICO |
| `native/ui/icons/` | 项目绘制的 SVG 界面图标 |

GUI 主线程只处理界面与模型。Tokio 使用两个工作线程，阻塞池上限八个。本地 PTY 使用独立读写及管理线程，关闭标签或取消任务时终止并等待回收对应进程。Windows 保留 PowerShell `-NoLogo -NoProfile`；Unix 选择可执行的绝对 `$SHELL` 路径并传入 `-l`，macOS 回退 `/bin/zsh` 再 `/bin/sh`，Linux 回退 `/bin/bash` 再 `/bin/sh`，设置 `TERM=xterm-256color`。

SSH 输出完整解析后，以最高约每秒 30 次更新屏幕。GUI 邮箱按会话合并画面，拥塞时跳过旧画面而不丢弃网络字节。文件列表虚拟化，终端模型仅替换变化的行。

输入和传输队列均有界。SSH 读取与写入并行，避免大段粘贴与回显相互等待。文件传输使用固定缓冲区，下载先写入同目录临时文件，再原子提交；失败或取消时清理临时文件。

主机地址、用户名和密钥路径是普通配置。Windows 密码与口令继续由当前用户 DPAPI 加密，原 Base64 格式不变。macOS/Linux 使用 `v1:` + Base64（12 字节随机 nonce、AES-256-GCM 密文及认证标签），认证附加数据固定为产品与格式版本；每次加密使用系统随机数生成 nonce。解密拒绝未知格式、篡改和错误主密钥，绝不回退明文。

macOS 随机 256 位主密钥存于默认 Keychain 的 generic password，以配置目录标识隔离；创建使用 add-only API，避免并发首次启动覆盖已有密钥。Linux 暂不依赖 Secret Service，采用配置目录内 `0600`、当前用户拥有的 32 字节 `master-key-v1` 文件：原子创建、不覆盖已有密钥、拒绝符号链接/不安全权限/损坏文件，解密不自动重建丢失密钥。配置目录不可由其他用户写入。此 Linux 回退只依赖文件权限，不防御已能读取当前用户文件的攻击者；主密钥须与配置一起安全备份。临时明文和主密钥缓冲区使用 zeroize。`--data-dir` 隔离主密钥位置或 Keychain 条目；不支持直接跨平台搬迁加密凭据。

主机配置导出会剔除凭据及信任指纹。新配置目录存在时，不会被旧版本的数据迁移覆盖。

首次连接需确认 SHA-256 指纹；保存的指纹变化后拒绝连接。快速连接输入只解析支持的 SSH 参数，不作为 shell 命令执行。

Windows 自定义标题栏拖动继续使用 Win32；Unix 使用 Slint 的 winit 0.30 window accessor 调用 `drag_window`。终端测量、输入法和显示共享平台等宽字体（Consolas / Menlo / DejaVu Sans Mono）及设置中的字号。Linux 文件选择启用 XDG portal backend，需要运行中的 desktop portal 服务及桌面实现。设置页打开配置目录分别使用 Windows `explorer`、macOS `open`、Linux `xdg-open`。Xshell 默认目录探测和系统主题注册表查询仅在 Windows 启用；Unix 不预选 Xshell 目录，System 主题回退为浅色。

`.github/workflows/ci.yml` 在 Windows、macOS、Ubuntu 上构建、测试及上传 release 可执行文件；交互式 GUI 测试保持 ignored。`native/scripts/package-macos.sh` 生成未签名 `.app`/DMG，`package-linux.sh` 生成 tar.gz、桌面文件及 PNG，均须在对应宿主执行。后续工作为真实桌面验证、发布者签名/公证及 Linux Secret Service 集成；CI 配置的存在不等于已通过平台验证。

本次 Windows 构建、测试、GUI 自检及交叉检查的边界见 [跨平台验证记录](cross-platform-validation.md)。
