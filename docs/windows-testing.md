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

### 文档引擎开发诊断

[Document Engine Development Checks](../.github/workflows/document-engine-check.yml)
是独立的定向开发入口。默认由 `workflow_dispatch` 手动触发；为让尚未进入默认
分支的新 workflow 首次注册，只对 `codex/desktop-next` 上修改该 workflow 文件
本身的 push 启用极窄 bootstrap。普通源码修改不会自动运行，也不添加 PR trigger。
此安排遵循 [GitHub 手动 workflow 的注册要求](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#workflow_dispatch)，不修改 main。

该入口准备 pinned 生产资源后，第一个 Cargo 测试只运行精确的 synthetic ACL
用例；失败即停并上传日志。通过后精确运行 Python、OfficeCLI 及逃逸拒绝的功能
前置，成功才串行执行全部 `windows_sandbox`、三个附件备份及长路径回归、三个
Host 中断／无副作用／未知结果回归，共九个固定命令。功能前置失败同样立即停，
避免其余用例重复重型预检。每个命令都检查实际测试
计数，零测试、跳过、失败或不完整输出均不能通过。开发 debug0 v3 缓存纳入
Cargo.toml 的 features/profile，兼容恢复旧 v2 依赖；断言失败仍保存已完成依赖。

任何 Cargo 命令之前先运行真实 PowerShell 控制通道预检，确认 stdin 静默时
仍可轮询取消和 deadline，并完整读取 STOP/CLEAN/EOF。专用入口要求恰好
4 pass、0 skip／fail／cancel 和唯一完整结果；工具缺失、子进程超时或只跳过
真实进程用例均不能通过。macOS portable PowerShell 的结果不能代替这一步。

日志及 `summary.json` 位于同 SHA、同 attempt 的
`document-engine-development-<SHA>-<attempt>` artifact，不含发布包或账户配置。
不运行全 sidecar、release probes、安装包或付费模型。默认 Windows Host pipeline
e2e 当前未纳入，能力开关关闭时保持未验证。这个结果不能代替完整 Windows Probe、
发布同 SHA 门禁、安装版 UI 或 Windows 10/11 客户端验收。

本轮 `0xc0000142` 启动定位的首次有界 ETW 采集已完成 owned stop 和原始文件
删除，但解码失败，不能据此排除访问拒绝或推断具体原因。后续只在上述 GitHub
Actions 开发入口，为精确功能用例的第一个固定可信 Node 使用已有 Microsoft
签名 CDB；本地入口保留原行为。诊断必须先完成原 token 校验，再核对实际
PID/TID、loader 标志读回和 primary thread 挂起计数，由原 Host 恢复执行。
不修改 IFEO、capability、ACL 或 mitigation，不将异常统一标为 handled。签名
SymChk 只为固定系统 ntdll 从微软公开库提前准备私有匹配符号；CDB 只加载本地
符号，不访问网络。attach 前与调试 continue 后各实测 `1→配对2`，再要求原
Host Resume1；GO 仅确认，不发送第二个 g。V2 完整证明缺任一项均不归因，
但允许真实失败早于 Host 确认标记被观察到。原始调试输出只写上传目录之外的
私有临时目录并清理，artifact 仅保留
有界脱敏记录；未知文案或握手失败保持 inconclusive。最后加载的 DLL 不能自动
认作故障原因。原 Job 和进程句柄仍负责终止、等待及取退出状态，调试器退出不
替代这些条件。调试模式始终停止在 Node 诊断，不产生引擎 admission；功能门禁
最终必须在没有调试器的情况下通过。

仅显式设置 `OFOX_DOCUMENT_STARTUP_COMPARISON`（`registryRead`、`detachedConsole`
或 `detachedRegistryRead`）且该固定用例已取得
完整原始启动证明、Node 返回 0142 时，测试代码才追加
无调试器的 A-B-A：同一临时 profile/输入，A1/A2 为该候选声明的基线启动，B 只改
一个条件（追加 registryRead 或去掉控制台）。A1 不能重现则不派 B；每组都须验证实际精确 token/隔离属性和完整清理。
候选成功也保留原用例失败，不改产品 capability 或 admission；没有 PID、SID、
路径和输出正文的闭集收据写入该用例日志，不替代完整功能或客户端验收。
原生运行 44027445443 的 A1/B/A2 均返回同一 0142，registryRead 单独不足；默认
诊断不会重复该候选实验。
同一对照另有 `detachedConsole` 候选：A1/A2 与产品相同使用 `CREATE_NO_WINDOW`，B 仅
改为 `DETACHED_PROCESS`，capability、token、策略和输入均不变；`DETACHED_PROCESS`
只存在于 test cfg，产品路径无法选择。开发工作流在该步骤显式设置此候选，回答
控制台初始化是否为 0142 的原因；答案明确后移除该设置。B 成功也不改产品启动
标志，修复须另行提交并通过无调试器的功能门禁。
各组回执另含闭集 `stderrShape`：仅 kind（empty、libuv-fatal、node-assertion、
node-fatal、v8-fatal、icu、openssl、unknown）；libuv-fatal 只在调用名属于固定
Win32/libuv 词表时附该名与 i32 数字错误码，否则二者均为 `?`/null；逐行查找
致命行，已知调用优先，不上传任何原文。
`detachedRegistryRead` 的 A1/A2 均以 `DETACHED_PROCESS` 启动，且必须精确复现
`WSAStartup` 10107 的 libuv 中止（`0x80000003`）；B 仅追加 registryRead。每个候选的
基线启动与预期失败由协议固定，A1 不复现则不派 B。开发工作流当前设置此候选。

固定 Nt 观察保留原 V2/Host Resume1 门禁。对实际 child HANDLE 调用
`IsWow64Process2`，要求成功、processMachine=UNKNOWN、nativeMachine=AMD64，
并核对原生 Windows 绑定的 size/offset；使用的是公开 Windows x64 ABI，不宣称
PDB 布局读回。原有 SymChk 与 gflag 实测仍保留。实际线程表与 `bl /L` 须证明全部固定
owned 入口断点（当前 15 个，含仅记录状态的 NtCreateUserProcess）已解析、启用并
关联实际线程。每次调用的 owned 持久返回断点 30
独立验证地址和线程，返回仅在同 TID、入口 RSP+8 和保存的返回地址上配对 EAX。
ACCESS_MASK 取 EDX 低 32 位；ALPC PortName 与注册表 ValueName 不作访问掩码。
每次属性调用先检查 OA 可读范围及 Length=0x30，US/Buffer 也先检查可读范围；
不可读或长度不符只将可选名字标为 unknown，仍配对真实返回状态。名字按
UNICODE_STRING.Length 读取，最多 1024 字节；相对 RootDirectory 不推断
绝对对象。原始名字/地址留在私有目录并清理，上传仅含固定 API、状态、掩码、闭集
对象别名，以及按固定词表投影的对象形态：每段名字只有与公开 Windows 组件词表
精确匹配（仅 ASCII）时才按词表拼写输出；首个词表外组件之后全部为 `?`，纯数字
（如会话号）只遮蔽自身，UNC 主机不入词表，超过 32 段为 `?overflow`。全部已配对调用按序列出（含成功调用），便于看到 DLL 失败前的最后操作。嵌套、缺返回、未安装或超过 256 次调用均 inconclusive；该上限内的
全部失败状态都保留，正常启动的大量“不存在”不能挤掉最后的失败操作。失败操作
不自动等于 DLL 初始化失败的原因。这些合成文案回归通过也不证明原生 CDB 兼容。
生成命令显式使用 MASM；布尔比较以 and/or 连接，不混入 C++ 的逻辑运算符。
CDB 会在提示符后回显 `-cf` 的每条命令；`bl` 解析跳过 `0:000>` 回显，带提示符
的行不能计为断点行。`-cf` 结束后 CDB 打印不换行的提示符等待 `g`，其后第一行
输出会接在提示符后；投影只在提示符后紧接 `OFOX_NT_RAW_V1 ` 时剥离并计数
`promptJoined`，另以闭集 `firstRejection` 报告首个拒绝原因，不含原文。按微软示例，当前 `bl` 在状态位后输出 DML 动作文字
`Disable Clear`；解析接受该文字及线程列的 `~` 前缀，仍要求启用、地址和实际线程号。
`OBJECT_ATTRIBUTES` 绑定只在 test cfg 使用，所需 `Wdk_Foundation` 以 Windows
dev-dependency 的同一 pinned `windows` 提供，发布构建的特性集合不变。

运行时测试入口会先准备完整生产资源，再验证打包依赖与清单；Windows Probe
同时安装根目录依赖，供 PPT 来源和覆盖率维护测试使用。上游 PPT 资源禁止 Git
换行转换，来源哈希始终核对原始字节。

英文 Windows CI 镜像没有 CJK 字体，因此缩略图像素测试使用固定提交及 SHA256
的 Noto Sans CJK SC（SIL OFL 1.1），仅下载到 runner 临时目录并在测试中注册。
这不证明未安装 CJK 字体的用户系统能正确回退，也不会把字体打包进应用。

资源库 MCP 检查的 Windows Job、取消后子进程回收、stdio/HTTP/SSE 也包含在完整 Rust 测试中。它们使用本机 Node 和临时模拟服务器，不执行已配置的真实 MCP。必须在 Windows 原生运行；macOS 交叉类型检查不能验收 Job 行为。实现边界见 [MCP 检查](mcp-connection-checks.md)。

## 用量与资源库补充验收

用量测试使用内存数据库或临时原生日志，不能用真实 settings 损坏、真实会话删除或工具卸载代替。完整 Rust 测试覆盖当前已接入 reader；自动化通过仍不表示每个原生 Agent 的 Windows 安装版都已对账。记录实际 Agent 版本、来源格式、目录覆盖和失败原因；未安装或没有可靠样本的工具明确标为未验。

收到源码验证包时，先核对随包清单的完整提交 SHA 和 SHA256，保留同一份源码执行上述 PowerShell 命令。包内不包含本机数据库、账户或原生配置。克隆仓库可用 `git rev-parse HEAD` 对照提交；解压源码包使用随包提交清单及压缩包摘要，不把另一个 checkout 的成功日志用于这份源码。脚本结果和界面截图一并保留，最终发布仍由同 SHA Windows Probe 门禁判定。

| 场景                                         | 期望                                                                             |
| -------------------------------------------- | -------------------------------------------------------------------------------- |
| 概览／会话分析／请求之间切换项目、模型与时间 | 共享条件保留，未知项目与未解析项目分开，缺归属明确说明                           |
| 请求附加状态、来源与搜索后翻页、详情和导出   | 摘要、趋势、测量样本、分页与完整 CSV 使用同一冻结条件；后台同步不替换当前页      |
| 原生日志带合成 200、0 耗时或旧首事件时间     | 合成状态和占位耗时不计入测量；没有可靠首 Token／生成起止时刻时显示未记录及零样本 |
| Hermes 会话分析                              | Hermes 标签、项目与活跃时长出现；会话继续后同一行原地更新，不重复计入            |
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

## 助手本机配置

Windows Probe 对 `feat/assistant-*` 分支启用完整 Rust、运行时和生产资源检查。
完整 Rust 测试包含 PowerShell 中文及空格路径、输出脱敏与截断、超时后禁止延迟写入，
以及权限白名单、敏感路径、加密快照、中断还原、双区域绑定和区域切换隔离。
运行时测试覆盖 Windows PowerShell operations 到 Rust 的回调转发。
排查时可单独运行（不能替代完整门禁）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib services::assistant::tools
```

Shell 执行使用系统 Windows PowerShell、`-NoProfile` 和 Windows Job。
测试使用临时目录及测试凭据，不需要启动桌面应用或调用付费模型。
macOS 上通过的路径策略和回调测试不能证明 Windows 原生进程行为；
复审时须核对同一提交的 Windows Probe 结果。文件还原保存只读属性，不保存完整 ACL。

macOS 另有默认忽略的真实 fnm 验收测试
`real_fnm_problem_folder_repair_verify_and_restart_restore_in_each_mode`。
设置 `OFOX_TEST_FNM`、`OFOX_TEST_NODE_OLD`、`OFOX_TEST_NODE_NEW` 和
`OFOX_TEST_OPENCODE` 为本机现有可执行文件后，用 `cargo test` 的
`--ignored --nocapture` 运行。测试复制两个 Node 和 OpenCode 到临时 HOME，
复现问题文件夹切换 Node 后找不到工具，并验证三档权限、修复、原文件夹验证和重启还原；
它不安装工具，不修改原有 fnm 目录。

原生凭据库验收由 Windows 检查脚本单独执行（不包含在默认忽略测试中）：

```powershell
cargo test --manifest-path src-tauri/Cargo.toml --lib os_keychain_restores_encrypted_snapshot_in_a_new_process -- --ignored --nocapture
```

测试用独立 Credential Manager 条目，让两个独立进程分别写入快照和读取密钥还原，
结束后删除测试条目。macOS 同名测试显式创建临时钥匙串，不依赖临时 HOME 的默认钥匙串，
不读取或修改应用已有密钥；生产 debug/prod 服务名的隔离由另一个单元测试约束。

Windows 助手命令保留继承的 `PSModulePath`，并将所用系统 PowerShell 的内置模块目录放在首位。原生 CI 探针复现了环境过滤后，继承 Windows PowerShell 5.1 模块路径时简单命令也可能挂起；内置模块优先后通过，其他模块目录仍保留。命令执行回归覆盖简单命令、Unicode 路径、限量输出与超时清理，环境过滤测试仍要求凭据变量被剔除。
