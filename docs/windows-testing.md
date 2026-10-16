# Windows 验证与界面验收

本次新增的 Windows 测试代码已就绪，Windows 原生运行结果待主机验证。自动化
测试使用临时配置、虚构 key 和受控进程，不需要先安装任何 Agent 或登录 OFox。
忽略的真实安装/升级测试不会被自动运行。

## 准备主机

使用 Windows 10/11 的 MSVC Rust 环境：

- Visual Studio Build Tools，勾选“使用 C++ 的桌面开发”和 Windows SDK；原生依赖
  需要 CMake 时也安装 CMake 工具。
- Rust MSVC 1.87+（仓库固定 toolchain 为 1.95），安装 `rustfmt` 和 `clippy`。
- Node.js 满足 Vite 7 要求（20.19+ 或 22.12+），pnpm 10.12.3。
- 运行桌面界面时需要 WebView2。

Windows 的 C++、WebView2 和 MSVC 环境要求见
[Tauri 官方前置依赖文档](https://v2.tauri.app/start/prerequisites/#windows)。

```powershell
rustup component add rustfmt clippy
```

在仓库根目录运行以下命令。脚本兼容 Windows PowerShell 5.1 和 PowerShell 7，
不需要管理员权限；`Bypass` 只作用于本次 PowerShell 进程。

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-windows.ps1
```

脚本依次安装锁定的前端依赖，执行 TypeScript、格式、前端单测、渲染构建、Rust
格式、Windows `clippy --all-targets` 和完整 Rust 测试，最后编译只读探针。
测试串行运行，配置目录隔离在临时目录；本地 HTTP 模拟测试临时绕过 localhost
代理，完成后恢复进程环境变量。

其他入口：

```powershell
# 已装好仓库依赖时，跳过 pnpm install
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-windows.ps1 -SkipDependencyInstall

# 仅验证 Rust/Windows 后端
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-windows.ps1 -BackendOnly

# 完整验证后生成可启动的正式配置模式 exe 和 release 探针
powershell -NoProfile -ExecutionPolicy Bypass -File .\scripts\test-windows.ps1 -BuildApp
```

`-BuildApp` 使用 `tauri build --no-bundle`，产物是
`src-tauri\target\release\ofox-desktop.exe`，不安装 MSI/NSIS 包。
测试入口按默认 `src-tauri\target` 查找探针；使用自定义 Cargo target 目录时可按
下方单独命令运行实际产物。

每次运行在 `artifacts\windows-tests\时间-PID\` 保存步骤日志和 `summary.json`。
退出码非 0 或 `success: false` 都表示未通过。排查时提供失败步骤日志和 summary；
不需要提供实际接入配置或 API key。

## 自动化覆盖

完整 Rust 测试包括安装证据分类、Windows `.cmd` 路径与版本异常、WSL/Shell
失败和超时、Appx 注册身份/版本、WorkBuddy 残留安装路径。绑定测试覆盖：

- 应用与配置独立；缺失文件不重建，读取/解析失败不当作缺失。
- Codex/ChatGPT 共享绑定、Gemini 部分文件缺失、WorkBuddy 恢复与解绑。
- `.ai`/`.io` 双向切换、首次启动、重启、模型保留及自定义地址冲突。
- 空格/中文 Windows 路径、CRLF 配置、独占文件锁导致的共享冲突。
- Windows 独占锁测试验证保存/恢复/解绑失败后文件与记录不丢失。

前端单测覆盖折叠区计数、按钮优先级、管理/恢复流程、仅新增项绑定、旧检测结果
不覆盖新操作，以及此前“已更新却仍可更新”的回归。

`.github/workflows/windows-probe.yml` 已使用相同后端入口。代码推送后，Windows
workflow 将构建两个探针并上传验证日志；全量前端检查继续由主 CI 执行。

资源库 MCP 检查的 Windows Job、取消后子进程回收、stdio/HTTP/SSE 也包含在完整 Rust 测试中。它们使用本机 Node 和临时模拟服务器，不执行已配置的真实 MCP。必须在 Windows 原生运行；macOS 交叉类型检查不能验收 Job 行为。实现边界见 [MCP 检查](mcp-connection-checks.md)。

## 用量与资源库补充验收

用量测试使用内存数据库或临时原生日志，不能用真实 settings 损坏、真实会话删除或工具卸载代替。完整 Rust 测试覆盖当前已接入 reader；自动化通过仍不表示每个原生 Agent 的 Windows 安装版都已对账。记录实际 Agent 版本、来源格式、目录覆盖和失败原因；未安装或没有可靠样本的工具明确标为未验。

收到源码验证包时，先核对随包清单的完整提交 SHA 和 SHA256，保留同一份源码执行上述 PowerShell 命令。包内不包含本机数据库、账户或原生配置。克隆仓库可用 `git rev-parse HEAD` 对照提交；解压源码包使用随包提交清单及压缩包摘要，不把另一个 checkout 的成功日志用于这份源码。脚本结果和界面截图一并保留，最终发布仍由同 SHA Windows Probe 门禁判定。

| 场景                                         | 期望                                                                             |
| -------------------------------------------- | -------------------------------------------------------------------------------- |
| 概览／会话分析／请求之间切换项目、模型与时间 | 共享条件保留，未知项目与未解析项目分开，缺归属明确说明                           |
| 请求附加状态、来源与搜索后翻页、详情和导出   | 摘要、趋势、测量样本、分页与完整 CSV 使用同一冻结条件；后台同步不替换当前页      |
| 原生日志带合成 200、0 耗时或旧首事件时间     | 合成状态和占位耗时不计入测量；没有可靠首 Token／生成起止时刻时显示未记录及零样本 |
| Hermes 累计弹窗                              | 独立累计范围，不随 7／30 天或项目筛选重分配；缺价、未归属、陈旧数据分别显示      |
| 原生文件移动、WAL 更新、源缺失或设置不可读   | 更新可验证的事实；失败保留旧结果并提示，不能切到默认目录报成功                   |
| 900px 浅／深色、键盘筛选与趋势、详情关闭     | 无横向溢出，图表左右键可查看值，关闭详情回到触发项，刷新保留焦点                 |
| 资源库预览、取消、应用与恢复                 | 只在专用沙箱操作；保留非受管内容，冲突阻塞，取消不改用户文件                     |

首 Token 的旧列表示首个 SSE 事件，不能直接当生成时间。当前用量实现的测量边界见[查询契约](usage-analytics-contract.md)，原生来源限制见[工具接线](agent-wiring.md)。不得因为界面有字段便将未记录指标补成零或推算速度。

## 在目标主机观察安装状态

只读探针可直接在待测账户运行。它调用真实安装检测和 `--version`，不执行
安装、更新、绑定或恢复，也不请求远端最新版本。报告省略原始进程错误、配置
内容和密钥；`unknown` 与 `notInstalled` 明确区分。

```powershell
# -BuildApp 的 release 产物，或 Windows workflow 的同名探针
.\src-tauri\target\release\verify_tool_lifecycle.exe
.\src-tauri\target\release\verify_tool_lifecycle.exe --tool codex --tool chatgpt --tool workbuddy
```

未使用 `-BuildApp` 时探针在 `target\debug\`，读取开发配置模式下的目录覆盖。
release 探针与正式桌面应用一致；报告中的 `profile` 用于确认模式。

探针退出码 0 仅表示检测完成，不表示每个工具都已安装或工作正常；逐项查看
`installationStatus` 和 `diagnostic`。WSL 检测使用同模式 OFox 设置里的 WSL
配置目录覆盖；未设置覆盖时检测的是 Windows 本地安装。它不会主动启动代理服务。

## Windows 界面验收

建议在专用测试账户绑定工具，并保存本次操作需要的配置备份。删除应用或配置
由测试人员主动执行；一键测试脚本不会做这些操作。安装后的实际连通性需要
测试账户的 OFox 登录与有效 key。

| 场景                            | 期望                                                        |
| ------------------------------- | ----------------------------------------------------------- |
| 只删除 WorkBuddy 应用，配置保留 | 刷新后进入“未安装工具”，计数保留，管理/解绑可用             |
| 同时删除应用与配置              | 未安装区可管理；解绑预览提示配置已删除；确认后不创建目录    |
| 已安装但删掉受管配置            | 主列表显示恢复绑定；保存/连通性测试禁用；恢复后保留已选模型 |
| Gemini 仅删除一个配置文件       | 状态缺失；恢复只补接入配置；解绑跳过已删文件                |
| 配置被改成自定义地址/模型/key   | 显示冲突，普通恢复不覆盖；管理中仍可预览解绑                |
| Codex 与 ChatGPT 同时绑定       | 解绑其中一个不改共享配置；最后解绑也不重建已删文件          |
| 安装存在但版本检查失败          | 留在主列表，显示修复/下载和管理                             |
| 配置目录指向不可达 WSL          | 检测失败留在主列表，显示重新检测，不能进入未安装区          |
| 全部绑定工具未安装              | 主区域显示暂无已安装工具，折叠区仍能操作                    |
| `.ai` → `.io` → 重启 → `.ai`    | 实际文件地址跟随区域；模型/key 保留；已删字段继续缺失       |
| 900px、中/英/日、深浅主题       | 操作可换行、无横向溢出，管理始终末尾；Tab/Enter 操作可用    |
| 更新已经完成                    | 刷新/focus 后没有旧版本对应的“可更新”提示                   |

每个恢复后的工具在管理中运行一次连通性测试，并检查实际配置里的受管地址与
选定区域一致。记录系统版本、应用版本、探针 JSON 和失败场景，配置中的 key
无需复制到日志或截图。

## 本轮会话工作台原生复验

使用最终SHA源码包，除以上完整门禁外，记录以下Windows原生结果：

- Codex普通日志与`archived_sessions`的发现、读取、Markdown导出；移动和副本不重复统计，归档不开放未经验证的继续／删除。
- 搜索先准备、订阅再运行，取消、继续扫描、预算限制及变化后的陈旧命中；OpenCode/Hermes SQLite与WAL替换必须拒绝陈旧内容。只在临时原生来源沙箱制造替换和损坏。
- 阅读器每次打开重新验证，目录／正文查找可定位虚拟化消息；长代码横向滚动，HTML作为文字，图片不自动联网。
- 900×600浅深色完整排行可滚动到分页，关闭恢复焦点；小时网格方向键与日历、会话／用量往返定位保留筛选。
- 实际账户切换与两个apex只在用户授权的测试账户验证。单测通过不冒充已登录Windows真机验证。

可单独记录缓存聚合release基准（会话／事实／消息规模、fixture限制和计时范围见实施记录）：

```powershell
cargo test --release --manifest-path src-tauri/Cargo.toml --lib session_analytics_release_warm_query_p95 -- --ignored --nocapture
```

该命令不读取真实会话，不能替代原生扫描或UI延迟验收。
