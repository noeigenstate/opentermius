# 跨功能集成测试与缺陷排查

## 已发现的缺陷

### Keychain 允许不同密钥占用同一 ID

- **症状**：通过 `Store::add_key` 加入两个指纹不同、ID 相同的密钥后，主机的 `keyId` 无法唯一确定密钥；`key_material` 只返回第一条匹配记录，而 `recovery::generate` 因重复 ID 拒绝生成恢复码。
- **原因**：新增密钥只检查指纹重复，没有检查 ID 重复。生成器通常产生 UUID，但库 API 也接受调用方提供的 ID。
- **修复**：在修改配置前拒绝已有密钥 ID，保留原有的指纹去重及保存失败回滚逻辑。
- **回归测试**：`duplicate_key_ids_are_rejected_without_breaking_host_or_backup`。先创建引用原密钥的主机，再尝试插入相同 ID 的不同密钥；要求拒绝插入，内存与磁盘不变，原密钥仍可解析，恢复码仍能生成。

修复前已实际运行该回归测试，结果为 0 通过、1 失败，失败断言为 `duplicate key IDs must be refused`。修复后的结果见下方验证记录。

## 覆盖范围

| 测试 | 验证内容 |
| --- | --- |
| `legacy_defaults_then_current_config_is_a_golden_roundtrip` | 不含新字段的版本 2 配置加载默认值；设置、主机、片段联合持久化；重新打开和再次保存的 JSON 字节不变 |
| `xshell_import_groups_and_dedup_survive_recovery` | 递归导入 Xshell 会话，保留分组，忽略供应商密码；同批去重；恢复后再次扫描仍去重，重新打开数据一致 |
| `populated_recovery_restores_secrets_files_and_preferences_in_both_modes` | 完整配置在 Merge、Replace 模式下恢复到已保存的空配置；密码与私钥重新加密、文件私钥原文件删除后可恢复、片段、转发、OpenSSH known_hosts 导入的信任指纹、主机视图及设置保留；重复合并不产生额外项目或文件 |
| `host_deletion_releases_key_but_preserves_detectable_orphan_forward` | 使用中的密钥无法删除且不改变磁盘；删除主机后可删除密钥；孤立转发仍保留主机 ID，可重新打开和备份恢复，缺失密钥返回错误 |
| `garbage_imports_and_recovery_are_bounded_and_do_not_mutate_config` | 固定种子的 64 组随机字节、无效恢复码、随机 known_hosts 行、超过 5 MiB 的 Xshell 文件、超过 20 MiB 的恢复码及 1 MiB 的无效 known_hosts 行；错误输入不改变已有配置 |
| `recovered_keychain_and_file_keys_forward_bytes_and_log_lifecycle` | 两种恢复模式下，从重新打开的配置中读取恢复后的 Keychain 密钥和文件私钥，实际向进程内 SSH 服务进行公钥认证，经本地转发往返传输 32 KiB 二进制数据，并停止隧道；日志 connecting、connected、closed 各阶段均重新打开验证 |

## 边界与限制

- 测试仅使用独立临时目录和 `127.0.0.1` 上的进程内服务，不依赖外部 SSH/SFTP 服务。
- DPAPI 及恢复后认证测试仅在 Windows 编译执行；没有跨 Windows 用户、跨计算机或其他操作系统的实机验证。
- ID 检查阻止新增重复记录，不自动修复此前已经存在重复 ID 的配置。
- 日志集成测试根据观测到的隧道状态显式调用 `LogBook` API，验证状态及 JSONL 持久化，不等同于验证 GUI 自动接收事件的全部路径。
- 删除主机测试验证库中孤立引用可检测、可持久化；页面启动转发时已有缺失主机错误检查，本次不改变 `.slint` 页面。
- 随机输入覆盖是固定规模的健壮性回归，不是完整模糊测试或内存峰值基准。恢复码、会话文件分别覆盖现有输入上限；known_hosts 测试验证给定大输入正常返回。
- 现有 `transport`、`forwarding`、`sftp_ops`、终端与本地 ConPTY 测试由全量测试统一执行。本次没有新增 SFTP 服务夹具或修改 UI。

## 验证记录

验证日期：2026-10-20，Windows x86_64 MSVC。所有 Cargo 命令在 `native/` 内运行，使用本工作树的 `native/target`，未新增依赖或修改锁文件。

| 检查 | 结果 |
| --- | --- |
| 修复前单独执行重复密钥 ID 回归测试 | 在预期断言失败，确认缺陷可复现 |
| `cargo build --locked` | 退出码 0，无警告 |
| `cargo test --locked` | 退出码 0，254 通过、0 失败、4 忽略，无警告；其中本次新增 7 项全部通过 |
| `cargo test --locked --test transport native_gui_live_selftest -- --ignored --nocapture` | 使用本工作树已构建的 debug 可执行文件，退出码 0，1 通过、0 失败，无警告；其余 3 项现有桌面专用忽略测试未单独执行 |
| `rustfmt --edition 2024 --check tests/integration_features.rs tests/integration_recovery_transport.rs` | 通过；仅检查本次新增测试文件 |
| `opentermius.exe --smoke-test --smoke-scene=section-keychain --data-dir=<fresh> --output=<output>` | 退出码 0，启动标记 `ok: true`；已查看 `native-window.png`，导航选中状态、页标题、空状态及按钮清晰，无裁切 |

除上述重复密钥 ID 缺陷外，本次覆盖的恢复、导入、设置持久化、引用删除、回环认证及转发流程未发现其他缺陷。
