# 字体、抗锯齿与圆角

## 测量方法

本次实测环境：Windows 10 Pro 22H2（19045），NVIDIA GeForce RTX 3090，驱动 32.0.16.1664，Slint 固定为 1.18.1。所有 Cargo 命令均在 `native/` 执行，沿用该工作树的 `native/target`，不改用新的目标目录。

使用正式 release 配置（thin LTO、strip、静态 CRT）。各渲染器分别编译；每次启动使用全新的临时数据目录。内存使用正常启动的应用，等待输入空闲后再等 3 秒，然后每 250 ms 采样一次、共 8 次，计算工作集与私有字节平均值；不启用烟测或截图，避免截图分配影响结果。首帧另启独立的 `--benchmark` 进程收集。每个版本各启动 3 次；汇总使用三次结果的中位数。GPU 显存及驱动进程内存不包含在进程内存中。本机同时有其他编译任务，数字属于本次环境的观察值，不能视为冷启动基准或长期稳定性保证。

首帧计时从进入 `main` 开始，到处理首次 `RedrawRequested` 后回到事件循环空闲处结束（包含窗口/上下文创建、字体初始化和首帧提交）。它写入 `first-frame.json`，不是截图模式固定等待的 800 ms，也不是屏幕实际发光时间。外部启动与就绪文件轮询时间另行保留在原始测试输出中。

| 版本 | Release EXE | 空闲工作集 | 空闲私有字节 | 首帧 | 100% / 150% 质量观察 |
| --- | ---: | ---: | ---: | ---: | --- |
| 原软件渲染基线 | 22.94 MiB | 23.11 MiB | 9.07 MiB | 无首帧仪表 | 中文可读；100% 按钮曲线的像素阶梯与小字边缘在放大图中可见，150% 文本较细 |
| 软件渲染、平台字体 | 22.97 MiB | 23.64 MiB | 9.23 MiB | 53.97 ms | Segoe UI / Cascadia Mono / 中文正常；仍可见软件曲线的像素阶梯 |
| FemtoVG OpenGL + 软件回退 | 23.46 MiB | 73.00 MiB | 101.29 MiB | 593.97 ms | 100% 小字清楚，150% 曲线与文字灰阶过渡自然；按钮、输入框边框无可见接缝，中文无缺字方框 |
| Skia OpenGL + 软件回退 | 未生成 | 不可测 | 不可测 | 不可测 | 构建失败，无截图；不能推断运行时质量 |

Skia 实验执行了 `cargo build --release --locked --no-default-features --features renderer-skia`。`skia-bindings 0.153.3` 请求的预编译产物 `b7f043e0b1e2a850e702-x86_64-pc-windows-msvc-d3d-ganesh-gl-jpegd-jpege-pdf-static` 返回 HTTP 404，随后源码构建报告 `Unable to locate LLVM installation`。因此本机没有 Skia release 可执行文件；表中不填写猜测数据，不提供伪造的 Skia 对照图。可选 feature 保留给有完整工具链或可用二进制缓存的诊断环境；默认构建不启用它。

## 选择与回退

默认选择使用平台字体的软件渲染器，以保持较低的内存占用和启动开销。FemtoVG OpenGL 作为可选 GPU 路径保留：Settings 的 Appearance 提供 “Smoother rendering (GPU)” 开关，提示 “Uses more memory. Takes effect after restart.”。偏好保存为 `settings.gpuRendering`，默认为 `false`；旧配置缺少此字段时仍使用软件渲染。开关只改变下一次启动，不在当前进程中替换渲染后端；恢复默认设置也会关闭它。

启动选择顺序为 **命令行 > 环境变量 > 保存的 GPU 偏好 > 软件默认值**。支持 `--renderer=software|femtovg|skia`、`--renderer software` 和 `OPENTERMIUS_RENDERER`；重复参数取最后一个，未知值明确报错。启动时先读取同一个配置存储（包括 `--data-dir` 指定的隔离配置），再创建渲染后端。GPU 偏好开启时请求 FemtoVG；请求未编译的渲染器时回退软件并输出诊断信息。

上表保留此前正式 release 的对照测量：相对平台字体的软件版本，FemtoVG 的空闲工作集约增加 49.36 MiB，私有字节约增加 92.06 MiB，首帧中位数约增加 540 ms。它改善了曲线和平滑程度，但不再作为默认。当前机器的三次首帧波动明显，不能视为冷启动或其他平台的保证。

Slint 在 Windows OpenGL 2.0 探测失败时可能自行选择软件；应用通过渲染通知能力识别这一情况。窗口或 GL 上下文创建的错误也可能延迟到事件循环启动时发生，因此不能仅包裹 `BackendSelector::select()`：应用识别首帧之前的平台错误，清理会话与运行时后，保留原参数再启动同一程序，并追加 `--renderer=software`。Slint 的平台是进程全局单例，这样无需尝试替换已安装的后端。软件进程不再重试，父进程等待并传递退出码。配置、文件与 SSH 错误，以及首帧后的错误不触发这条启动回退路径。

烟测的 `startup.json`（包括实时 SSH/SFTP 自检）和 `first-frame.json` 记录实际使用的 `backend`。发生进程重试时，`startup.json` 的 `pid` 保留启动进程身份，`rendererPid` 标识实际绘图进程；子进程首帧计时从子进程的 `main` 开始，完整重试耗时应看外部启动计时。父进程等待退出时仍有自身的内存开销。截图继续使用 `Window::take_snapshot()`，在当前渲染器读取画面，不通过切换软件渲染器生成 GPU 对照图。

## 字体与边缘

- Windows 界面指定 Segoe UI；在 Slint 的共享字体集合中为 Han 脚本指定已安装的 Microsoft YaHei UI。其他文字继续使用系统的字体回退链，不打包或下载字体。
- Windows 终端优先选择字体集合中存在的 Cascadia Mono，否则使用 Consolas，包含用户安装字体的检测。macOS 查询系统 UI 字体并用 Menlo 显示终端；Linux 使用默认 sans-serif 与 DejaVu Sans Mono。
- `Theme.ui-font`、`Theme.mono-font` 统一字体入口。终端用于测量的隐藏文本、输出、光标、选择、输入法位置和列数仍共享实际测量的字宽；自检不再将 Consolas 的固定数值当作其他字体的正确宽度。
- 保留已有字号、400/500/600/700 字重、圆角设计 token 与布局。对照图检查按钮、输入框、卡片的边缘和边框接缝，不为抗锯齿修改设计圆角或添加整像素偏移。

Windows 原生无边框窗口显式启用 winit 阴影，设置 DWM 非客户区渲染，并扩展 1 px 框架以保留系统阴影。Windows 11 请求 `DWMWA_WINDOW_CORNER_PREFERENCE = DWMWCP_ROUND`；Windows 10 不支持该属性时忽略返回值，应用仍可启动。窗口快照只包含客户区，不包含 DWM 阴影或屏幕外框。本次 Windows 10 不能证明 Windows 11 原生圆角的实际外观。

## 截图对照

以下裁剪均来自实际 `--smoke-test` 的 `Window::take_snapshot()`。100% 原图为 1240×800，150% 原图为 1860×1200。裁剪使用同一逻辑坐标：100% 以最近邻放大 3 倍，150% 放大 2 倍，保留像素覆盖信息，不加锐化或平滑滤镜。终端 100% / 150% 的实际字宽均记录为约 8.225 逻辑像素，使用 Cascadia Mono；原 Consolas 约为 7.7。

| 内容 | 原软件基线 | 平台字体 + 软件 | 平台字体 + FemtoVG |
| --- | --- | --- | --- |
| 控件 / 100% | [放大图](screenshots/rendering-baseline-100-section-gallery.png) | [放大图](screenshots/rendering-software-100-section-gallery.png) | [放大图](screenshots/rendering-femtovg-100-section-gallery.png) |
| 控件 / 150% | [放大图](screenshots/rendering-baseline-150-section-gallery.png) | [放大图](screenshots/rendering-software-150-section-gallery.png) | [放大图](screenshots/rendering-femtovg-150-section-gallery.png) |
| 终端 / 100% | [放大图](screenshots/rendering-baseline-100-section-terminal.png) | [放大图](screenshots/rendering-software-100-section-terminal.png) | [放大图](screenshots/rendering-femtovg-100-section-terminal.png) |
| 终端 / 150% | [放大图](screenshots/rendering-baseline-150-section-terminal.png) | [放大图](screenshots/rendering-software-150-section-terminal.png) | [放大图](screenshots/rendering-femtovg-150-section-terminal.png) |
| 中文 / 100% | [放大图](screenshots/rendering-baseline-100-hosts.png) | [放大图](screenshots/rendering-software-100-hosts.png) | [放大图](screenshots/rendering-femtovg-100-hosts.png) |
| 中文 / 150% | [放大图](screenshots/rendering-baseline-150-hosts.png) | [放大图](screenshots/rendering-software-150-hosts.png) | [放大图](screenshots/rendering-femtovg-150-hosts.png) |

另附最终 GPU 版本的[完整控件页 100%](screenshots/rendering-current-gallery-100.png)、[完整控件页 150%](screenshots/rendering-current-gallery-150.png)及[完整终端页](screenshots/rendering-current-terminal-100.png)。检查了常规/中等/半粗/粗体层级、中文、密码输入、禁用状态、错误输入框、按钮圆角与卡片边缘。

## 重现与交付检查

```powershell
# 从 native/ 运行；不设置新的 CARGO_TARGET_DIR。
cargo build --release --locked --no-default-features
cargo build --release --locked --no-default-features --features renderer-skia
cargo build --release --locked
cargo build --locked
cargo test --locked
$env:OPENTERMIUS_TEST_EXECUTABLE = (Resolve-Path target/release/opentermius.exe).Path
cargo test --locked --test all transport::native_gui_live_selftest -- --ignored --nocapture

# 每次使用独立数据目录与输出目录；再用 SLINT_SCALE_FACTOR=1.5 重复。
$env:SLINT_SCALE_FACTOR = '1'
target/release/opentermius.exe --smoke-test --smoke-scene=section-gallery --data-dir=<fresh-data> --output=<output>
target/release/opentermius.exe --smoke-test --smoke-scene=section-terminal --data-dir=<another-fresh-data> --output=<another-output>

# 仓库根目录：
./native/scripts/package.ps1 -SkipBuild
```

Skia 的 `skia-bindings` 构建脚本可能下载 GitHub 上的预编译原生库，缓存未命中时还可能进入耗时的源码构建；它依赖网络、匹配的工具链和平台特定产物，不宜强加给轻量默认构建。默认 FemtoVG 构建增加 Slint 自带的 OpenGL 渲染依赖，不增加应用直接依赖；运行时不访问网络服务。现有 Linux CI 已安装 X11/Wayland/OpenGL 的桌面构建依赖。macOS/Linux 构建与 GUI 仍须由对应 CI 和桌面验证，本机 Windows 结果不代表这些平台已经验证。

本次验证结果（2026-10-20）：

| 检查 | 结果 |
| --- | --- |
| 软件独立 release / 默认 FemtoVG release | 均成功，最终无编译警告 |
| `cargo build --locked` | 成功，无编译警告 |
| `cargo test --locked` | 成功，无编译警告；默认忽略交互式 GUI 测试 |
| release `native_gui_live_selftest` | 成功；实际 `winit-femtovg`、Cascadia Mono，覆盖真实 SSH/SFTP、指纹确认、DPAPI、持久化与截图 |
| 渲染器诊断 | CLI 覆盖环境变量成功；未编译的 Skia 回退软件成功；未知渲染器退出码为 1 并写错误文件 |
| 图片检查 | 查看了 100% / 150% 的 `section-gallery`、`section-terminal`、Hosts 中文及 editor；另外查看了实时 SSH/SFTP 自检截图 |
| `package.ps1 -SkipBuild` | 成功，`OpenTermius-0.2.0-windows-x64-setup.exe` 为 7,706,619 字节 |
| `verify-install.ps1` | 成功；临时安装、覆盖重装、快捷方式、注册表、静默卸载、用户配置保留及清理均通过 |

新增 5 个单元测试，覆盖 CLI/环境变量优先级、无效/缺失参数、编译 feature 对默认选择的影响、Cascadia/Consolas 选择、仅首帧前 GPU 平台错误触发一次软件重试。当前 GPU 正常工作，真实 RDP/旧驱动故障现场尚未复测；Windows 11 的原生外框、macOS/Linux 的构建及桌面表现也不属于本次 Windows 10 实测结果。Skia 的构建限制见上文。

## UI 后续修正（2026-10-20）

本轮使用 debug 构建验证软件默认值和深浅主题，前面的 release 性能、打包及安装记录保留为上一轮测量。本轮未重新打包发布产物。

- 共享单行 Input 的编辑区域覆盖控件高度，并显式垂直居中；窄输入框的可用宽度下限为零。Field、SearchField、端口和 SFTP 路径栏共用此修正。
- Logs 表头固定为表格完整宽度，保留与数据行相同的列比例和重连操作槽。
- SFTP 进度条固定从左侧零点开始，宽度按限制在 0–100 的百分比计算；`section-sftp` 提供断开状态的 42% 视觉样例，不启动传输。`section-forwarding-editor` 提供含 8080/5432 端口的编辑器样例。
- Settings 保留 main 的分组布局和共享组件，在 Appearance 中加入 GPU 偏好开关；字体提示显示实际平台等宽字体。其他页的初始页面、控件画廊及主机编辑器已检查深浅主题。

| 检查 | 本轮结果 |
| --- | --- |
| `cargo build --locked` / `cargo test --locked` | 成功，无编译警告；281 项通过，4 项交互桌面测试默认忽略 |
| `transport::native_gui_live_selftest -- --ignored --nocapture` | 成功，软件渲染；真实回环 SSH/SFTP、指纹、DPAPI、持久化及截图 |
| 渲染器选择 | 单元测试覆盖默认值、CLI/环境变量/设置优先级和既有 GPU 失败重试条件；6 次实际启动检查保存的 GPU 偏好、两种覆盖和未编译 Skia 的软件回退 |
| 偏好设置 | 旧配置默认关闭 GPU；持久化使用 `gpuRendering`；指针点击切换、恢复默认及重开配置均已测试 |
| 视觉检查 | 原分支检查了 11 个场景 × 深浅主题，均实际使用 `winit-software`；合并后重新检查 Settings 深浅主题，GPU 提示和平台字体名称完整显示 |

| 页面 | 浅色 | 深色 |
| --- | --- | --- |
| 端口转发编辑器 | [截图](screenshots/ui-followup-forwarding-editor-light.png) | [截图](screenshots/ui-followup-forwarding-editor-dark.png) |
| Logs 完整表头 | [截图](screenshots/ui-followup-logs-light.png) | [截图](screenshots/ui-followup-logs-dark.png) |
| Settings（合并后） | [截图](screenshots/settings-light.png) | [截图](screenshots/settings-dark.png) |

合并后的 Settings 截图来自完整原生窗口，使用软件渲染和 Cascadia Mono。GPU 初始化故障在本机未实际复现；软件重试判定沿用已有测试。其他平台及 Windows 11 外框仍需对应环境验证。
