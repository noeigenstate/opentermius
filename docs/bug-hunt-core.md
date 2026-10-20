# OpenTermius 核心模块缺陷排查报告

- 日期：2026-10-20
- 生产代码修改：`native/src/store.rs`、`native/src/terminal.rs`。
- 未修改 UI、SSH/SFTP、ConPTY 实现，未引入依赖。

## 复现与修复

临时将两个生产模块换回 `HEAD` 原始代码，在同一 `native/target` 下执行三个新增集成测试目标（`--no-fail-fast`）：Quick Connect 6 个失败、Store 11 个失败、Terminal 2 个失败，共 19 个失败；随后恢复工作实现。以下修改均有失败用例支撑。

| 问题与原因 | 修复 | 回归测试 |
| --- | --- | --- |
| 导出没有检查目标是否为活动配置，会覆盖凭据和信任记录 | 拒绝相同路径及 `canonicalize` 后指向活动配置的路径；测试同时检查文件字节未变 | `export_refuses_to_overwrite_the_live_configuration` |
| `persist` 没有尺寸检查，可写出超过打开上限的配置 | 序列化后检查 `< 16 MiB`，再写临时文件；超限保留旧文件 | `persist_refuses_to_write_a_configuration_that_could_not_be_reopened` |
| 字段只检查 `max * 4` 字节，允许超长 ASCII 文本 | 增加字符数限制：名称、用户名、分组 128，地址 253 | `validate_limits_text_fields_by_characters_not_only_bytes` |
| 地址允许内部空格，直到连接阶段才报错 | 拒绝地址中的空白字符，复用既有控制字符检查 | `validate_rejects_control_characters_and_whitespace_in_hosts` |
| 密钥路径未修剪、未限长；纯空白路径通过密钥认证校验 | 修剪首尾空白，限制 32767 字符并拒绝控制字符 | `validate_accepts_only_known_auth_types_and_requires_a_key_path`、`validate_rejects_oversized_or_controlled_private_key_paths` |
| `trust` 可写入空主机、空指纹和异常空白 | 检查主机、指纹的非空性、空白、控制字符和长度 | `trust_rejects_empty_hosts_and_fingerprints` |
| Quick Connect 把 `host:port` 整体当主机，无法正确拆分 `[ipv6]:port` | 分离地址和端口，检查括号结构和数字端口；显式 `-p` 优先，裸 IPv6 保持完整 | `parses_host_colon_port_and_bracketed_ipv6`、`rejects_invalid_hosts_ports_and_brackets` |
| `-i ''` 因路径为空而静默切换为密码认证 | 单独记录是否指定 `-i`，空路径交给密钥认证校验拒绝 | `rejects_invalid_hosts_ports_and_brackets` |
| 复制软换行文本总插入换行并裁掉有意义的行尾空格 | 快照记录 vt100 的逐行软换行标记；软换行直接拼接，硬换行保留原行为 | `selection_across_soft_wrapped_rows_has_no_line_break`、`selection_across_soft_wrapped_rows_keeps_intentional_spaces` |
| 导入的 5 MiB 上限低于配置的 16 MiB 上限，有效导出无法导回 | 导入与打开、持久化统一使用 `< 16 MiB`；导入主机数上限仍为 10000 | `a_valid_export_larger_than_five_megabytes_can_be_imported_again` |

续跑发现初稿的新字符限制存在回归：空名称会自动填入最长 253 字符的完整主机地址，导致重复校验失败；Quick Connect 也会拒绝这种地址。两个补充用例先分别复现失败，再将自动显示名称限制为前 128 个 Unicode 字符，保留完整连接地址：

- `default_display_names_for_long_hosts_survive_revalidation_and_reopening`
- `long_valid_hostnames_do_not_exceed_the_display_name_limit`

## 新增测试与覆盖

Windows 下共新增 **75 个测试**：67 个集成测试和 8 个模块内测试。

| 文件 | 数量 | 覆盖内容 |
| --- | ---: | --- |
| `native/tests/store_core.rs` | 33 | 字段、端口、认证、Unicode、缺失/损坏/超限配置、原子写入与失败回滚、增删改、信任指纹变化、导入导出、重复条目和 ID、DPAPI |
| `native/tests/terminal_core.rs` | 21 | CJK、emoji、组合字符、16/256/真彩色、样式、光标、备用屏幕、滚动、缩放、DA/DSR/CPR、输入模式、长行、分块/非法 UTF-8、选择文本 |
| `native/tests/quick_connect.rs` | 13 | `user@host`、`ssh`、`-p/-l/-i`、IPv6、地址端口、引号、Windows 路径、Unicode、长地址、错误参数和注入样式输入 |
| `store.rs` 末尾 `core_regression_tests` | 5 | 缺失配置、迁移条件、损坏/版本错误的旧配置、迁移失败保留源文件、DPAPI 密文迁移 |
| `terminal.rs` 末尾 `core_regression_tests` | 3 | 3000 行历史上限、64 KiB 响应上限、软换行标记 |

未发现新问题的覆盖包括：保存密码和口令的 DPAPI 往返及篡改拒绝、认证类型变更时清除旧凭据、失败写入不遗留临时文件、指纹变化拒绝、导出移除信任与密文（`encryptedSecret` 为 `null`）、导入重新生成 ID 且不覆盖已有主机，以及既有的 SSH/SFTP 回环测试和真实 PowerShell 生命周期测试。

## 已知限制

- 旧配置中超过新字符上限的用户字段会被拒绝，不自动截断；只有空名称的自动回退会缩短显示名称。
- `main.rs` 的全选复制仍使用 `snapshot.lines.join("\n")`，跨软换行保留换行；本轮修复鼠标选择路径。
- 宽字符在只剩一列时换行，vt100 未提供软换行标记，该情况仍复制出换行；测试记录了现有行为。
- Quick Connect 的 Windows 反斜杠路径需要单引号，实际地址解析仍由连接层完成；解析器不执行 shell 输入。
- 导入重复条目不会去重，每条均生成新 ID；响应缓冲超过 64 KiB 后丢弃新增回复，均保留既有策略。
- 未新增下载中途取消及私有 `main.rs::safe_filename` 专项测试。既有传输测试覆盖成功下载、拒绝覆盖及临时文件清理。
- `export` 不单独限长；直接修改公开 `data` 后导出的超大内容仍可能无法导入。

## 最终验证

所有 Cargo 命令在 `native/` 执行，使用工作树原有 `native/target`；等待共享构建锁，未清理锁文件或终止其他进程。

- `cargo build --locked`：退出码 0，无编译器警告。
- `cargo test --locked`：退出码 0，96 个通过、0 个失败、1 个交互式 GUI 测试默认忽略，无编译器警告。
- 原生 GUI 自检：直接运行上述 Cargo 构建的 `transport` 测试程序，选择 `native_gui_live_selftest --ignored --nocapture`，并指定最终 debug 可执行文件及独立输出目录；1 个通过。11 项检查覆盖认证、SHA256 提示、真实 SSH 回显、SFTP、标签清理、DPAPI、持久化、搜索及字体度量；截图已查看。
- 最终构建的 `section-known-hosts` 冒烟检查退出码 0，`startup.json` 为 `ok: true`。1860×1200 截图已打开检查：导航、文本、尺寸正常，无可见裁切；页面为既有占位页，未新增页面功能。
- 截图保留在忽略目录 `native/test-output/core-final-9e9b20a56fcd45d197ad3e3cfb7a06b4/` 的 `section-known-hosts/` 和 `gui/` 子目录，文件名均为 `native-window.png`。
