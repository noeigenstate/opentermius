# 跨平台验证记录（2026-10-20）

验证宿主为 Windows x64，Rust 1.97.1 MSVC、Slint 1.18.1。所有 Cargo 命令均从 `native/` 执行。没有推送代码或触发远端 CI。

| 检查 | 结果与边界 |
| --- | --- |
| `cargo build --locked` | 最终源码通过，退出码 0，无新警告；默认产物为 `target/debug/opentermius.exe` |
| `cargo test --locked` | 最终源码通过，退出码 0，无新警告；23 项通过、1 项交互式 GUI 测试按设计忽略 |
| `cargo test --locked --test transport native_gui_live_selftest -- --ignored --nocapture` | 另行通过，退出码 0；真实原生窗口、认证/指纹提示、SSH 回显、SFTP、DPAPI、配置持久化、标签清理及字体度量 |
| GUI 截图 | 已查看 `native/test-output/port-gui/native-window.png`，布局、终端文字、光标与文件列表正常；单元格宽度约 7.70 DIP，保持 Consolas 14px 度量 |
| Bash 与 ShellCheck | 两个 Unix 打包脚本通过 Bash 语法检查及 ShellCheck 0.11.0；Windows 打包脚本通过 PowerShell 语法解析 |
| `aarch64-apple-darwin` 完整应用 `cargo check --locked` | 最终源码通过，退出码 0，无新警告；使用 Zig 0.16.0 交叉 C 编译，仅验证类型/元数据，没有生成并运行 macOS 应用 |
| `x86_64-unknown-linux-gnu` 的 `cargo check --locked --all-targets` | 最终源码通过，退出码 0，无新警告；使用 Zig 和临时 fontconfig 链接元数据，涵盖库、应用及测试代码；没有验证原生链接或执行 |
| Linux/macOS 凭据模块及测试代码 | 用临时检查入口直接引用实际 `secrets.rs`，Zig 配合两个目标的 `cargo check --tests` 均通过；没有在 Unix 上执行测试 |

已安装并尝试 `x86_64-unknown-linux-gnu` 和 `aarch64-apple-darwin` 目标。默认 Linux 全量交叉检查阻塞于 `yeslogic-fontconfig-sys 6.0.1`：此 Windows 宿主没有配置支持交叉编译的 pkg-config/Linux fontconfig sysroot。Ubuntu CI 已配置 `libfontconfig1-dev` 等原生开发依赖；宿主构建和测试仍须由 CI 验证。

为继续检查 Linux Rust 代码，在忽略的临时 Cargo 配置中声明了下面的链接元数据，跳过系统库探测并保留正常的静态 FFI 接口。这足以运行 `cargo check --all-targets`，不证明系统库存在或可链接；项目和 CI 的默认配置没有这一覆盖。

```toml
[target.x86_64-unknown-linux-gnu.fontconfig]
rustc-link-lib = ["fontconfig"]
```

Zig 检查仅在命令进程中配置目标专用 `CC`/`AR`、`CRATE_CC_NO_DEFAULTS=1`（避免 cc 自动追加 Zig 不支持的 Rust target triple）以及同盘的 Zig 缓存目录。临时工具、检查入口、交叉编译配置及构建缓存均在忽略的 `native/target/` 内，不进入提交，也不改变默认构建方式。

新增逻辑测试覆盖 AES 的空值/Unicode 往返、随机 nonce、错误密钥、篡改、未知格式和长度限制；Unix 配置目录权限、Linux 主密钥权限/损坏/符号链接/并发创建/丢失密钥；shell 选择与回退，以及忽略 SIGHUP 后的取消回收。原有终端集成测试在 Unix 上另检查真实 PTY 尺寸和 `TERM=xterm-256color`。Unix 专属测试的实际执行仍待 CI。

尚未验证 macOS/Linux 真机 GUI、macOS Keychain 实际授权与持久化、两个 Unix 打包脚本的实际执行及原生 release 链接。打包产物未签名/公证。Linux 主密钥回退为用户拥有的 `0600` 文件，并未使用 Secret Service；失去该文件就无法恢复已保存凭据。CI 文件已添加，尚无本分支的远端执行结果。
