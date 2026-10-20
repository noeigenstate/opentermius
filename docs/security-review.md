# 本地 SSH/SFTP 客户端安全复核

范围：`native/src/**`，Windows/MSVC，重点检查凭据、信任边界、文件写入、输入解析和资源限制。测试使用临时配置及进程内回环 SSH/SFTP 服务，不依赖外部网络服务。本报告不是对恶意本机管理员、进程内存读取或所有第三方依赖的安全保证。

## 发现与修复

| 编号 / 严重性 | 攻击场景 | 修复 | 回归测试（`native/tests/all/security.rs`） |
| --- | --- | --- | --- |
| S1 / 中 | 恶意 SFTP 目录项使用 `COM¹`、`LPT²`、`CONIN$` 等 Windows 设备别名，绕过已有普通设备名过滤。旧 `Download` 命令也未采用新下载路径的文件名检查。 | 补全设备别名规则；两种下载入口均在创建临时文件、打开远程文件之前验证目标名。 | `windows_download_names_reject_device_aliases_and_path_escapes`、`loopback::legacy_download_rejects_unsafe_names_before_remote_open` |
| S2 / 高 | 选择的本地上传路径含目录 junction/reparse point 或文件链接时，单文件上传会跟随链接，读取所选目录之外的文件。目录扫描后文件被替换成链接也可能发生同类问题。 | 打开上传文件时重新检查路径及所有祖先，拒绝链接和非常规文件；Windows 最终文件使用 `FILE_FLAG_OPEN_REPARSE_POINT`，并检查已打开句柄元数据。旧、新单文件入口及递归上传共享检查。 | `loopback::uploads_reject_junction_ancestors_before_opening_remote_files` |
| S3 / 中 | 调试终端输入命令或屏幕事件时，派生 `Debug` 输出密码字节或终端回显；失败断言也可把这些内容写入 panic 文本。 | 为传输命令、事件实现不携带负载的 `Debug`。 | `terminal_input_and_screen_debug_do_not_disclose_secrets` |
| S4 / 中 | 畸形 PuTTY 私钥解析错误包含输入行；恢复数据反序列化错误可回显载荷字段；SFTP 服务端状态文本可以回显认证密码，并经错误链进入状态栏。 | 私钥和恢复数据解析只返回固定错误；SFTP/自定义 I/O 错误在传输消息边界返回固定中文提示，保留本地校验提示。 | `malformed_private_key_errors_do_not_echo_input`、`malformed_recovery_errors_do_not_echo_payload_secrets`、`loopback::sftp_errors_never_echo_server_credentials_and_remove_partial_downloads` |
| S5 / 中 | Xshell 所选根目录为 junction 时会读取外部会话；深度和总目录项缺少显式上限。深层会话可能先被分组长度校验拒绝，但这不能限制扫描递归本身。 | 扫描限制为最多 64 层、10,000 项；所选根目录也拒绝链接；维持单文件 5 MiB 上限和只读操作。 | `xshell_scan_refuses_selected_junction_root`、`xshell_import_is_read_only_and_caps_recursion`、`xshell_import_caps_total_entries` |
| S6 / 中 | 每条转发有连接上限，但规则数量及配置恢复后的自动启动数量未显式限制。 | 新增规则上限 64 条，允许修改已有规则；界面启动同样限制同时运行规则数量，避免导入配置绕过。 | `forwarding_rule_count_is_bounded_but_existing_rules_can_be_edited`（规则校验），已有 forwarding 测试覆盖运行、停止及连接上限 |

修复前执行新增回归套件，19 项中 10 项失败、9 项通过。原有恢复目录 junction 拒绝测试已通过，因此没有将其列为漏洞，也未修改该保护。深度测试在此后加强为验证明确的深度拒绝原因，避免误将分组长度校验当作递归限制。详细最终结果见下文验证记录。

## 已检查且未发现新的绕过

- **凭据保存与导出**：主机密码、Keychain 私钥及口令经 DPAPI 加密，序列化字段使用 `encrypted*` 名称；主机导出清除凭据、整个 Keychain 和信任记录。`saved_credentials_exports_and_recovery_respect_secret_boundaries` 验证持久化和导出不含测试明文，恢复只有明确包含秘密时才携带可恢复凭据。已有 `keychain` 测试覆盖带口令的私钥导入、解密与导出。
- **日志**：GUI 的 `pages/logs.rs::failure_summary` 只存固定错误分类，不记录原始服务端错误、终端 I/O 或认证答案；已有 GUI 模块测试验证回显密码不会进入 `opentermius-logs.jsonl`。日志模块是接受调用者文本的存储层，不对任意人为写入的标签或自定义错误作语义脱敏。
- **主机密钥**：终端与 SFTP 共用 SSH 会话校验；未知指纹等待明确同意，取消、拒绝、超时或指纹变化均不继续认证。转发共用相同校验，要求已有信任记录。`loopback::untrusted_keys_block_terminal_sftp_and_forwarding_before_authentication` 验证终端、排队 SFTP 和转发拒绝时，服务端认证次数与文件打开次数均为零。已有 transport 测试覆盖明确同意与取消信任提示。
- **指纹导入与恢复**：`known_hosts_conflicts_never_replace_existing_pins` 验证导入、再次信任及持久化不会覆盖不同指纹；已有 recovery 测试覆盖合并、替换模式仍保留本机冲突指纹。
- **恢复密钥路径**：安全 ID 禁止路径分隔符，写入固定的配置 `keys` 子目录；目录链接拒绝、独占创建、冲突后缀及失败回滚已存在。`recovery_refuses_junction_keys_directory_without_external_writes` 验证当前 Windows/Rust 的原有检查可拒绝 junction，且不在外部目录写入文件、不修改配置；已有 recovery 测试覆盖遍历 ID 与持久化失败回滚。
- **下载与清理**：逐组件拒绝 `..`、分隔符、绝对路径、盘符、UNC、ADS、控制字符、尾随点/空格；递归下载先验证所有目录项，拒绝已有链接目录。临时文件使用独占创建，失败/取消清理，Windows `MoveFileW` 提交不覆盖已有文件。新测试验证远端失败后的临时文件清理；已有 `sftp_ops` 覆盖恶意目录项、递归下载、取消清理和不覆盖已有文件。
- **输入**：快速连接通过 shlex 解析为 `Profile` 后交给 russh，不启动 shell；本地 PowerShell 启动参数固定。`quick_connect_has_no_shell_side_effects_and_forward_defaults_are_loopback` 检查命令注入形态无副作用、转发默认监听 `127.0.0.1`；`0.0.0.0` 必须显式填写。剪贴板与 snippet 的 bracketed paste 都移除负载 ESC；`bracketed_paste_cannot_break_out_and_query_replies_are_bounded` 验证 breakout 和累计应答上限 65,536 字节。
- **SOCKS5**：版本、认证方式、CONNECT 命令、保留字节、地址类型、域名、非零端口均有验证；握手由调用方施加超时。`malformed_socks5_inputs_return_errors_without_panics` 覆盖截断、变异头、空方法/域名和无效 UTF-8；已有 forwarding 测试覆盖合法 IPv4/IPv6/域名及连接并发上限。
- **资源限制**：每会话命令队列 64、文件任务队列 8、写入队列 8；每条转发最多 256 活跃连接；目录操作最多 100,000 项；日志最多 1,000 条且读取文件小于 16 MiB；配置/主机导入小于 16 MiB，单次主机导入最多 10,000；Keychain 最多 500，单私钥文件最多 1 MiB；恢复码最多 20 MiB、解压后最多 12 MiB，每列表最多 10,000。`oversized_imports_logs_and_recovery_are_rejected_before_processing` 验证外部输入上限；已有测试验证队列背压、日志裁剪和恢复认证/压缩边界。

## 限制与后续事项

- 路径元数据校验与后续打开/创建之间仍存在祖先目录被恶意本机进程并发替换的竞态窗口；没有声称实现整个目录树的句柄相对、原子无链接遍历。恢复密钥目录检查同样受此限制。
- `Zeroizing` 已用于主要私钥、密码、恢复序列化/压缩缓冲区；Slint 字符串、第三方认证 API、终端历史和队列字节可能保留短暂副本，不能通过单元测试证明整个进程内存已擦除。终端按用户操作发送、显示的秘密也不属于错误消息脱敏的保证范围。
- 不带恢复口令的恢复码内嵌解密密钥，持有码即可读取明确包含的秘密；这是既有便携备份格式。显式私钥导出和恢复后的 `keys` 文件可能是明文；必须按秘密文件保管。此轮不改变文件 ACL 模型。
- 验证平台仅为 Windows/MSVC；未验证其他平台、恶意本机管理员或被修改的第三方依赖。

## 验证记录

- 修复前回归：19 项中 10 项失败、9 项通过；修复后 19 项全部通过，无编译警告。合并测试后的等价命令为 `cargo test --locked --test all security::`。
- 原生截图：使用独立临时数据目录运行 `--smoke-test --smoke-scene=section-known-hosts`，退出码 0；已查看 `native/test-output/security-review/smoke/native-window.png`，页面标题、导航、导入与搜索控件、空状态完整可见，无裁切或错误覆盖层。
- `native/` 中 `cargo build --locked`：退出码 0，无警告；`cargo test --locked`：退出码 0，合计 266 通过、0 失败、4 项桌面测试默认忽略，日志中无编译警告。完整测试再次执行并通过全部 19 项安全回归（包括加强后的深度拒绝和远端错误确实返回的断言）。
- 额外执行编译后的 `transport` 测试程序：`--ignored --exact native_gui_live_selftest --nocapture`，1 项通过、退出码 0，验证当前 debug 可执行文件的原生 GUI 回调、实际 SSH/SFTP、DPAPI 及截图。其余 3 项默认忽略的桌面测试未单独执行。此轮未验证 release 安装包。
