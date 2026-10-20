# OpenTermius 架构

OpenTermius 使用 Rust、Slint 和 Tokio。当前 Windows 版本采用 winit 窗口后端和软件渲染，发布配置启用 thin LTO、符号剥离及静态 CRT。macOS 和 Linux 是后续平台适配目标。

| 模块 | 职责 |
| --- | --- |
| `native/ui/main.slint` | 主机、编辑器、终端、认证提示和双栏文件界面 |
| `native/src/main.rs` | GUI 回调、会话状态、事件聚合、剪贴板及文件选择 |
| `native/src/store.rs` | 本地配置、主机指纹、DPAPI 与配置版本迁移 |
| `native/src/ssh.rs` | SSH 认证、通道、SFTP、文件传输与取消 |
| `native/src/local.rs` | ConPTY 与本地 PowerShell 生命周期 |
| `native/src/terminal.rs` | vt100 屏幕、历史、颜色、查询回复及文本选择 |
| `assets/` | 应用标识和生成的 PNG/ICO |
| `native/ui/icons/` | 项目绘制的 SVG 界面图标 |

GUI 主线程只处理界面与模型。Tokio 使用两个工作线程，阻塞池上限八个。本地 ConPTY 使用独立读写及管理线程，关闭标签或取消任务时回收对应进程。

SSH 输出完整解析后，以最高约每秒 30 次更新屏幕。GUI 邮箱按会话合并画面，拥塞时跳过旧画面而不丢弃网络字节。文件列表虚拟化，终端模型仅替换变化的行。

输入和传输队列均有界。SSH 读取与写入并行，避免大段粘贴与回显相互等待。文件传输使用固定缓冲区，下载先写入同目录临时文件，再原子提交；失败或取消时清理临时文件。

主机地址、用户名和密钥路径是普通配置。保存的密码与口令由当前 Windows 用户的 DPAPI 加密，没有明文回退。主机配置导出会剔除凭据及信任指纹。新配置目录存在时，不会被旧版本的数据迁移覆盖。

首次连接需确认 SHA-256 指纹；保存的指纹变化后拒绝连接。快速连接输入只解析支持的 SSH 参数，不作为 shell 命令执行。

后续跨平台工作包括默认构建目标、系统凭据存储、本地 shell、字体与终端度量，以及各平台的签名、打包和 GUI 验证。目前没有宣称 macOS 或 Linux 已验证可用。
