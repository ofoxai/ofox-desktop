# 桌面助手稳定性与跨平台验收记录

跟踪：[Fizzy #1701](https://app.fizzy.do/6179973/cards/1701)。基线
`b84a4d574b057bb73972b4f6adb502f830ef3208`，版本保持 1.5.2。
本轮处理卡片前两批，不发布版本、不迁移数据库、不改真实用户配置。

## 稳定性修复

- 工具修改后的验收按 `(tool, canonical folder)` 记录。修改 Codex 后验证 Claude
  不再清除待验收状态；新修改令旧验证失效。Host 在副作用前检查 32 个目标容量，
  确定无副作用的失败释放预留。不完整快照保留 `scopeIncomplete`，不猜测身份。
  仅内存延续，不声称重启后恢复任务；保留现有 shell-only 意图与执行配额政策。
- runtime 的 home 在任务开始前规范化，覆盖部分修改异常没有结构化结果的情况。
- 附件自动保存失败后，回复不再显示完成。保留成功附件、正文、原模型错误及费用
  证据；停止时不自动交付。同一 run 的 `fromPreview` 重试复用原字节，不重新运行
  Python；没有增加跨 run 的预览恢复存储。

RED 证据：跨工具误放行、附件落盘失败仍 completed、Stop 未置取消通道时仍交付、
home 别名及新错误码缺少文案均取得失败断言，再实施修复。

## 历史样例

只读取历史错误类别，测试使用合成公开内容，不提交原会话或用户文件。

| 样例                   | 当前结论                      | 验证                             |
| ---------------------- | ----------------------------- | -------------------------------- |
| 历史公式 `\sin` 不支持 | 现有实现已覆盖，不重复修改    | `python-deck-sincos.test.mjs`    |
| 历史缺少冻结计划       | 当前契约正确拒绝              | `presentation-planning.test.mjs` |
| 正常公式生成对照       | 核对真实 OPC、lint、OfficeCLI | 同一公式测试                     |
| 正常冻结计划对照       | 合法规划可生成，陈旧规划拒绝  | 同一规划测试                     |

两文件本机合计 46/46 通过。历史“有输出”不等于质量或安装版验收通过。

## Windows 与真实模型验收

资源打包不代表允许执行。LPAC/Job 适配器需证明文件、网络、子进程及低完整性
映像限制；公开 runner 必须匹配完整运行时摘要的成功探测证明，资源改变重新验证。
原生 CI、安装版和真实模型结果分开记录，入口见[一键验收说明](document-engine-client-acceptance.md)。
Windows 10 22H2、Windows 11 客户端结果均为完整验收必要条件；Server CI 和
deck 降级成功不能代替 Python 客户端验收。
长路径回归覆盖资源、备份恢复、读取与发布；不声称支持任意超过 MAX_PATH 的
进程工作目录，系统无法启动时仍拒绝 Python。

本机 Node 22.23.4、Codex CLI 0.166.0、Claude Code 2.1.317；版本命令只读。
Codex 原生配置检查使用临时 HOME；Claude 仅核对版本，不能声称其连通性已验证。

真实模型为两模型、三场景、macOS/Windows 11 共 12 项任务，Windows 10 另作
六份冻结输入原生重放。总预算 US$10、单任务 US$1；共享账本及主机额度、请求前
预留、未知费用停止。入口默认离线，不借日常账户 key；专用测试 key 缺失时保持
pending，尚未执行付费请求。

## 本机检查结果

macOS 26.5.1（25F80），Rust 1.95.0；以下均未调用付费模型：

- TypeScript、Prettier、renderer 构建、Rust fmt 和全目标 Clippy 通过。
- 前端：3372 通过、1 原有跳过；sidecar：828 通过；搜索服务：102 通过。
- Rust 默认全量：4555 通过、28 忽略。随后显式运行三个真实引擎用例，3/3
  通过，包含 OfficeCLI/Typst 生成、布局修复、Python 仅修改第 70 页并预览。
  Python 用例使用当前公共资源的私有副本，没有改动安装版或真实用户文件。
- 跨工具、取消/超时、裸未知结果归一、目录在 callback 前或等待 key 期间换向
  的真实 Host 回归全部通过；安装的 Codex 只在临时配置目录做原生配置验收。
- Windows 首轮修正三个编译错误，第二轮修正附件读取分支的 Clippy 问题。
  第三轮（`6edc4cb4`，Windows Probe 43872249231 attempt 2）进入原生功能测试：
  3994 通过、11 失败、12 忽略。失败暴露 raw Win32 超长路径、缓存 ACL generic
  权限映射和目录句柄发布契约问题；包含两项备份恢复失败，不能算作验收通过。
  修复保留 reparse/hardlink 拒绝、精确 ACL 和按目录句柄发布，需再次原生运行。
- macOS 验收包已由干净提交 `1cb01fd3` 构建，版本 1.5.2，ad-hoc 签名校验通过；
  尚未替换 `/Applications` 安装版，也未完成界面验收。
  Windows 客户端界面及真实模型验收仍待各自结果，不能用以上 macOS 数字代替。
  原开发提交均保留，主目录另一批进行中的改动未被本任务修改。

后续原生诊断（Windows Probe 43882483572，`12ebb621`）：Server 2022
build 20348 的八组实测中，Win32 相对目录句柄发布均报错 87，native
`NtSetInformationFile` 四组均成功且目标字节一致。实现据此保留目录锚定并改用
native class 10，调用和完成状态都必须为零；产品原生回归仍待新运行。
该轮 sidecar 因测试快照误用 POSIX 根相对路径而失败（730 通过、1 失败、97
跳过），未进入 Rust。测试已改为临时目录下的真实绝对路径，未放宽生产验证范围。

macOS 复跑还发现测试用例漏关 OfficeCLI 自更新，固定资源从 1.0.160 漂移到
1.0.161，引发并发读取 bundle 失败；已补齐兄弟用例的跳过更新参数和临时 HOME。
生产 runner、prepare 和 smoke 原已有这些限制。保留的 `1cb01fd3` 验收包仍为
固定 1.0.160，二进制摘要与下载 pin 一致；不是安装版验收通过的替代证据。

`329edf85` 的后续 macOS 全量结果：前端 3372 通过、1 跳过，Rust 4555
通过、28 忽略，Clippy/类型/格式检查通过。sidecar 出现一次退出清理观测失败
（827 通过、1 失败），原 trace 只有布尔值，不能据此断言是实际存活或观测降级。
保持生产清理与所有断言不变，增加仅测试用的 PID/ps 状态诊断后，工作稿全量
833 通过，搜索 102 通过；原偶发失败仍未定因，不声称已经修复。

Branch Packages 已补充独立的 Windows release Host driver artifact：编译前后
核对干净源码，从 Cargo JSON 绑定唯一测试 exe 和实际编译 SHA/clean，再记录
二进制摘要。六项打包回归及十二项 campaign 回归合计 18/18 通过，独立审查通过。
完整 installer、离线探针、开发 Host driver 与安装版 UI 是不同证据；详见
[客户端说明](document-engine-client-acceptance.md)。原生 artifact 构建仍待 CI。

Windows Probe 43885065018（`329edf85`）已完成：库测试 4008 通过、14 失败、
12 忽略。长路径附件恢复及目录句柄新建/替换发布实测通过，原 302/87 错误已消失；
缓存 ACL 前置检查仍拒绝，连带隔离、取消和证明用例尚未进入有效路径。另一个
中断用例的失败是测试入场用了 Windows verbatim spelling，而生产 admission 使用
`scope::resolve`；夹具与精确预期已统一，macOS 针对用例通过，Windows 待复跑。
未放宽 ACL 验收或中断债务断言，Windows Python 能力继续关闭。

为缩短原生定位，新增独立 Document Engine Development Checks：先精确运行
ACL 前置，失败即停；通过后运行完整 sandbox 及指定附件、Host 回归，拒绝零测试
与跳过。权限诊断只保留数值控制位和脱敏 ACE 形状，不记录 SID 或原始 SDDL。
该开发入口不提供发布证明；最终仍须完整 Windows Probe 和两种客户端验收。

开发运行 43895706755（`c8003270`）的精确 ACL 用例取得原生 RED：实际控制位
`0x9814`，模板 `0x9014`，四个 ACE 形状相同，仅相差
`SE_SACL_AUTO_INHERITED (0x0800)`。现只在自有描述符副本的比较键中归一化该
记账标记，真实对象、原始快照、完整 SID/权限条目、保护位、标签和其他控制位
保持严格校验；增加实际 FW/Low 标签/无保护/错 capability/额外 ACE 等拒绝用例。
双独立审查及模块/测试的 Windows 目标 Clippy 通过（macOS 交叉检查）；产品原生
GREEN 仍待下一运行。
开发入口另加精确的功能隔离前置，再运行整个 sandbox，避免同一前置失败反复
消耗测试时间；九个命令均需真实执行且不得跳过。

开发运行 43899245981（`dfe5a835`）的精确 ACL 用例已原生通过（含真实权限
负例）。紧接的功能用例仍失败，148.55 秒仅有通用 unavailable，不能据此判断
某个子进程超时或 LPAC 不支持；后续按门禁停止。现增加仅可信固定探针的阶段、
耗时、退出码、超时、输出字节数及固定检查点诊断，不回传原始输出或改变成功条件。

同时独立复现并修复 Windows Python 模块路径缺陷：Pyodide 的 POSIX MEMFS
误用 native `path.join`，会写入带反斜杠的字面文件名，随后 `import deck` 失败。
实际写入改为 POSIX join，真实 OS 路径保持 native。真实 MEMFS Windows/POSIX/
当前主机回归 4/4、既有真实 PPTX 生成 1/1 通过；尚不能把该缺陷认定为上次
原生失败的根因。资源已重新准备，双独立审查及模块/测试 Windows 目标 Clippy
通过，下一次原生功能验证仍 pending。

开发运行 43906191942（`8d1573f4`）再次确认精确 ACL 用例原生通过；功能
用例的首个失败阶段为 `node_isolation`，未取得 EngineOutput 或具体原生错误，
Python 和 Office 阶段尚未执行。运行前资源验证耗时 76,346 ms，Node 阶段
32,098 ms；后者包含缓存与资源摘要核验，不能据此认定子进程或管道超时。
下一次只沿用开发入口的精确前置用例，补充可信固定探针的底层闭集阶段及数值、
布尔证据以定位错误出口；隔离判断、错误码、时限、权限和能力开关保持不变。
专用 env 已就绪，尚未验证 API 授权，也未执行付费模型任务。

开发运行 43913114420（`440649cb`）的底层证据将失败锁定在 Resume 前的
`token_container`：普通 AppContainer 标志为 1；class 29 长度查询返回 4 字节，
class 46 的 NULL/0 查询返回长度 0，因此尚未取得 LPAC 标志的值。缓存所有权、
profile 路径/标签、进程创建和 Job 分配均已完成，task/profile 清理均成功；
失败不属于 Python、Office、进程执行超时或管道排空。下一次仅在相同 suspended
token 上增加固定缓冲查询及数值错误码对照，原来的失败关闭判定保持不变。

开发运行 43916502977（`1e009434`）完成同 token 对照，确认当前 Server 2022
build 20348 不接受 class 46 的用户态查询：Win32 NULL/4/8 字节均返回
`0x80070057`；同样的 Nt 查询均返回 `0xc0000003 (STATUS_INVALID_INFO_CLASS)`，
长度与缓冲均未写入。class 29 的正控长度为 4，固定 4/8 字节查询均成功且值为 1。
因此当前根因为使用了该系统不支持的 token 查询项，不能靠增大缓冲解决，也不能
把查询失败当成 LPAC=false。修复使用同一未恢复进程的 token 授权行为进行验证：
仅 Everyone、AAP、foreign capability 拒绝，exact runtime capability 允许；
API 失败、权限输出不完整或任一矩阵不符都拒绝。原有包/capability/完整性级别、
Job 和 mitigation 检查保留；普通 AppContainer 成对反控前置到原功能用例，均不
恢复执行。Windows 目标交叉 Clippy、observer 状态回归 4/4、格式及双独立审查
通过；实际原生成对控制和后续完整功能探针仍待验证，Windows 能力保持关闭。
该行为证明依据 [LPAC 语义](https://learn.microsoft.com/en-us/windows/win32/api/winnt/ne-winnt-token_information_class)
与 [AccessCheck](https://learn.microsoft.com/en-us/windows/win32/api/securitybaseapi/nf-securitybaseapi-accesscheck)
组合，不声称微软提供了专用的 IsLPAC Win32 查询契约。

开发运行 43921430351（`2de81c59`）中，前置的真实 suspended LPAC/普通
AppContainer 成对控制均已通过，随后 bundled Node 的全部 token/mitigation
校验与四项 AccessCheck 矩阵也通过。Node 恢复后约 2 ms 以 `0xc0000142
(STATUS_DLL_INIT_FAILED)` 退出，尚未输出任何 JavaScript 检查点；不是此前的
token 查询失败，也没有进程超时或管道排空失败。Job active=0、task/profile
清理成功。当前仅能确认启动初始化失败，具体 DLL/系统资源原因待最小原生诊断；
Python、Office 和其余回归仍未进入，Windows 能力继续关闭。

下一次定位保留原样启动与安全条件，只添加可信 PID/TID 观察及开发 CI 的私有、
有界 ETW。以文档化 XML 解码 THREAD、ImageLoad、FileIO/OpEnd、Registry，
不捕获 PROCESS 的 SID/命令行，也不导出原始 trace。独立审查发现的 opaque IRP、
跨进程 TID 归属和采集超时清理竞态均已补回归，pinned Node 下针对测试 29/29
通过。此结果验证采集逻辑与脱敏，不代表 PowerShell/ETW 原生已运行；下一次
原生证据仍 pending。未调整 capability、资源 ACL、mitigation 或引擎超时。

开发运行 43932638031（`f97e97fb`）确认原样 Node 仍以 `0xc0000142` 退出。
私有 ETW 会话启动与 owned stop 原生成功，临时原始文件已删除且没有强杀采集器；
但后续解码失败，只能报告 inconclusive，尚无具体失败操作可据以修权限。实际
工具清单确认已有 Microsoft 签名 CDB/gflags 10.0.26100.4202；controller 位于
session 2 且 UserInteractive=true，不能将故障归因于非交互 session 0。
下一步仅用已有 CDB 为同一可信 suspended Node 捕获进程内 loader snaps，完整
验证 token 后、核对目标与标志读回后才允许原 Resume；调试器结果不作为功能验收。

CDB 接线仅在 test cfg 的首个固定 Node 使用：原挂起线程计数必须为 1，配对
探测恢复后由原 Host Resume 返回 1，再发送普通 `g`。取消、配置不符或未知文案
均拒绝；原 Job 停止后先等待 owned debugger 退出，再排空标准流。私有 raw
日志保留至读取和 CLEAN 确认，失败阶段与 cleanup 状态分别保存。调试执行即使
成功也强制停止，不写 admission。pinned Node 的控制器／投影组合测试 27/27、
旧 ETW 投影回归 13/13、纯 Rust fence 4/4、observer 4/4、Windows 目标模块及
测试 all-targets Clippy 和格式检查通过。合成日志不证明 CDB 原生文案兼容，
`-pb` 下首次 Resume 前的真实握手及本轮原生结果仍 pending。

运行 [43943128991](https://github.com/ofoxai/ofox-desktop/actions/runs/43943128991)
（`d40e36df`）的完整日志和诊断产物已核对：ACL 1/1 通过，原 Node 的 token
与四项 AccessCheck 仍通过；随后 `cdb_handshake` 约 30 秒、`cdb_stop` 约 30 秒
超时，配置、flags 和 Resume 全部为空。控制器最终在取消后启动 CDB，stdout、
stderr 均 0 行；Node 侧确认 owned debugger 退出及 raw 删除。这次没有恢复原
Node，不能据此判断其 DLL 初始化故障。Python/Office 和后续回归尚未进入。

控制通道存在已复现的同步阻塞：Microsoft 文档明确 `Console.In.ReadLineAsync()`
同步执行。原调用在返回 Task 之前等待 Node 的 STOP，而 STOP 要等 Cargo 完成后
才发送；此前的超时和晚启动时间线与此一致。改为持久的普通 StreamReader 读取
自有 stdin，保留预读的 STOP/CLEAN，并在已有 target 和实际启动前检查取消。
同样的 ETW 控制通道共用该 reader。参见
[API 合同](https://learn.microsoft.com/en-us/dotnet/api/system.io.textreader.readlineasync)。

macOS ARM64 的官方 portable PowerShell 7.7.0／.NET 11.0.0 实进程回归先得到
3 fail／1 pass，修复后 4/4 通过；包括静默 stdin 下取消与 deadline、合并发送的
STOP/CLEAN/EOF。专用预检还强制校验唯一完整 TAP、4 pass、0 skip／fail／cancel，
避免零项真实检查退出 0；该门禁回归 2/2 通过。相关 Node 测试合计 46/46、PS
语法检查和独立复核通过。以上不是 Windows 证据。下一次只验证 Windows 控制
reader、同一 suspended Node 的实际 CDB 握手与 loader 失败记录，不改隔离条件。

运行 [43998374426](https://github.com/ofoxai/ofox-desktop/actions/runs/43998374426)
（`cc42947e`）确认 Windows 实进程控制预检 4/4、0 skip，ACL 1/1；CDB 握手约
15.2 秒后拒绝，清理等待降为 11 ms。控制器报告 `target_not_seen`，stdout
50 行／2554 字节、stderr 0 行；配置和原 Resume 仍为空，owned debugger 和 raw
清理成功。worker 的 `target_exited` 可能来自调试器清理导致附属进程退出，不能
当作未恢复 Node 的真实 DLL 失败证据。原握手标记尚未得到认可，Node 原因仍未知。

下一次只定位 CDB 启动协议：将启动选项置于 `-p PID` 目标之前，符合
[官方语法](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/cdb-command-line-options)。
这并未单凭文档证明旧顺序必错。新增的固定形态计数仅区分 prompt、标记提及、
数值 TARGET、gflag 值和 NUL；另保留最多 8 个显式 Win32 uint32 错误码。它们
不含原文、不参与 READY 或根因认定。失败记录先于 kill 发布，关闭确认仍须
等实际完成。真实 PowerShell AST 执行生产行解析器，证明 prompt／回显不能置
targetSeen，完整严格序列才配置；两项回归由 RED 修至 2/2 GREEN，独立复核通过。
不扩大权限、不提前 Resume；新原生问题是命令文件是否执行，以及原握手为何
未匹配。尚未取得 Node loader 失败的具体模块或操作。

运行 [44003216099](https://github.com/ofoxai/ofox-desktop/actions/runs/44003216099)
（`94f47bf3`）仍为 control 4/4、ACL 1/1，配置未完成且原 Resume 为空。
CDB 40 行／1952 字节，但 prompt、标记提及、数值 TARGET、gflag、NUL 全部为
0，已识别格式的 Win32 错误码为空；不能把这解释成没有错误。握手约 15.1 秒，
清理 22 ms，owned debugger 和 raw 清理成功。启动选项规范化没有解决命令
不执行的问题，不能将旧参数位置认定为原因。首失败已正确保留 `target_not_seen`。

下一步只改变调试器停止事件：`-pb` 保留，增加 `-xe ld:ntdll.dll`，请求在
attach 回放已有 ntdll 的加载事件时停止，使 CF 有机会在原线程仍挂起时执行。
[DebugActiveProcess](https://learn.microsoft.com/en-us/windows/win32/api/debugapi/nf-debugapi-debugactiveprocess)
记录已有 DLL 事件回放；[事件筛选](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/controlling-exceptions-and-events)
支持 `ld[:Module]`，其默认只输出消息。这不保证当前 CDB 的组合行为，仍须
原生确认。未加入 `-pr`、`-g`、`-o`、`gh`，不改变 token、权限或异常 handled；
原完整身份、flags、尾标记和挂起计数门禁保持。目标仅为验证能否在 Suspend=1
时完成 CF，成功后才读取实际 Node 启动记录；不是功能验收或权限修复。

运行 [44006150325](https://github.com/ofoxai/ofox-desktop/actions/runs/44006150325)
（`84bf8551`）首次执行了 CF：数值 TARGET 1 行、prompt 5 行，握手约 1.8 秒后
明确拒绝为 `gflag_before_unreadable`，不再等待 15 秒。control 4/4、ACL 1/1、
token/AccessCheck 仍通过；flags、配置、原 Resume 均为空，Python/Office 未进入。
owned debugger 与 raw 清理确认，stop 72 ms。TARGET 被解析不等于整套身份门禁
已通过；没有原 Node loader 故障证据。

解析器另有回归已复现：原来只接受八位 flags，合法短 DWORD（`0`、`2`）会被
拒绝。改为整行匹配一至八位十六进制，仍拒绝溢出、负数、尾随文本，并保留
before/after 差值与全部身份/挂起计数门禁。这只是兼容性修复，尚不能归因为本次
原生失败。两固定 gflag 块另记录 symbols/types/PEB/globalFlags/failure/extension
六项词法布尔值，不含原文，不参与 READY，不据此单独认定原因。下一次定点
原生检查只回答 flags 能否读回，或明确读回阻塞的类别；不增加超时、不降低隔离。
真实 PS 解析回归先失败、修复后相关 Node 48/48、0 skip；另覆盖
`GetExpression: not found: ntdll!NtGlobalFlag` 的词法观察，独立复核通过。

运行 [44010319498](https://github.com/ofoxai/ofox-desktop/actions/runs/44010319498)
（`395dcb83`）仍拒绝 gflag 读回，握手 1.43 秒、清理 72 ms；control 4/4、ACL
1/1、token/AccessCheck 通过。六项提示中 symbols/types/PEB/globalFlags/failure
为 true、extension 为 false，未识别任何数值 flags；短 DWORD 不是此处阻塞的
解释。原 Resume 仍为空，owned debugger/raw 清理成功，没有 Node loader 原因。

后续只检验匹配的 ntdll 符号是否影响实际读回：在 controller-ready 与任何 target
创建前，Microsoft 签名 SDK SymChk 对固定 System32/ntdll.dll 从微软公开库准备
私有缓存，`/pt`、单文件 PASSED、唯一完整汇总及退出码均须成立。参见
[官方选项](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/symchk-command-line-options)
与[完整匹配下载说明](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/using-symchk)。
含类型信息不保证所需字段可读，更不是 Node 根因结论。

SymChk 与 CDB 同属自有 Job，环境清空；准备有 30 秒/取消界限，双流按小块读取，
行 4 KiB、总 64 KiB 上限覆盖无换行输出。只取唯一普通、单链接、64 MiB 内的
ntdll.pdb，拒绝 reparse 祖先，再用稳定只读句柄复制到私有平坦目录。CDB 仍禁用
网络符号、正常匹配 PDB，完整身份/flags/挂起计数与 READY 门禁不变；准备失败
不启动探针。所有原生符号准备及读回结果待定。真实 portable PS 子进程负例、
C# 声明编译和布局检查、脱敏投影等相关回归 50/50、0 skip。

运行 [44012886591](https://github.com/ofoxai/ofox-desktop/actions/runs/44012886591)
（`97c8e1dc`）control 4/4、ACL 1/1；签名 SymChk 10.0.26100.4202 实际启动、
退出码 0、关闭确认，但未满足严格输出校验，拒绝为 `symbol_check_incomplete`。
本次未启动功能 Cargo 或 Node，准备约 6 秒、owned stop/raw 删除确认；不能把
退出 0 当成符号准备通过，也不能推断下载或类型失败。旧摘要仅在完整通过后保存
汇总，因此无法分辨编码、输出通道或单项/汇总行差异。

下一次只补实际格式证据：两流的字节/BOM/NUL/换行统计、严格行计数与唯一汇总值。
最多八行固定词表 token，普通数字、路径、SID、未知文字均替成 `?`；不导出原文。
形态统计不改变 UTF-8/stdout 严格验证或任何通过条件。真实 UTF-8 与 UTF-16 字节
夹具证明 BOM/NUL 只能被观察、不能授予成功；相关 51/51、0 skip。

运行 [44014718403](https://github.com/ofoxai/ofox-desktop/actions/runs/44014718403)
（`67179adc`）实际 stdout 130 字节，BOM/NUL 均无，CR/LF 各 4；stderr 为空。
两个汇总行唯一且为 failed=0、passed+ignored=1；单项行具有精确 ntdll.dll 与
PASSED 状态，后面附带详情，因此旧状态行尾假设未匹配。问题在本次新增准备
验证器的行格式，并非已证明的下载、编码或类型失败；Node 未启动。

现共享的单项模式保留精确记录前缀、目标与 PASSED 词，允许其后同一行详情；
详情不作为证明。唯一单项、完整汇总、exit0、PDB 文件/匹配与实际 gflag/挂起
门禁均保留。实际详情形态回归先 RED 后 GREEN，伪状态、外来模块、前缀回显、
重复单项仍拒绝；17 种实进程模式及相关回归 51/51、0 skip。

运行 [44015969522](https://github.com/ofoxai/ofox-desktop/actions/runs/44015969522)
（`2cb13cde`）符号准备与私有 PDB 复制通过，实际 PID/TID、flags `0→2` 及 CF
尾标记首次全部确认。随后 `cdb_hold` 测得 SuspendThread previous=2，旧门禁
预期 1 而拒绝；没有配对 Resume 或原 Host Resume。握手约 2.04 秒、清理 61 ms，
control 4/4、ACL 1/1、owned stop/raw 删除均确认。调试停止时测得 2，不足以
单独证明该额外计数的来源，更不能直接把 2 当作放行条件；原 Node 根因仍未知。

测试诊断改为分开证明两种暂停：在 target/attach 前实测原 hold `1→配对2`；
身份和 flags 确认后只发送一次普通 g 释放调试停止，保留原手动 hold。依据
[ResumeThread 文档](https://learn.microsoft.com/en-us/windows/win32/api/processthreadsapi/nf-processthreadsapi-resumethread)，
手动挂起的线程不因调试 continue 而恢复。随后在同一 30 秒总界限内重新测到
`1→配对2` 才允许原 Host Resume1；中间的 2 只允许撤回本次探测增量，不能授权。
异常计数、非配对返回、取消和过期均拒绝，产品 token/capability/权限不变。

GO 现在只确认实际原 Resume，不再发送第二个 g；严格整数身份和实际 final hold
一同确认。它可能晚于 Node 的 2 ms 失败、CDB 退出或清理 abort 被观察到，故
持续有界 drain，并以完整 V2 全局证明加 continue 区段归因；缺少原 Resume1
的 READY 或输出不能归因。包含该竞态、布尔/字符串计数拒绝、伪造/重复证明的
相关 Node 56/56、0 skip，纯 fence 7/7、observer 4/4；模块及测试的 Windows
目标 Clippy、全仓 Rust fmt、PS 解析及 Prettier 通过。交叉检查不是原生证明，
新暂停流程和原 Node loader 失败仍待定点原生确认。

运行 [44020847288](https://github.com/ofoxai/ofox-desktop/actions/runs/44020847288)
（`42f2edbb`）已原生证明整条诊断链：pre-attach `1/2`、flags `0→2`、5 次
release 探测后 fresh `1/2`、原 Host Resume1；握手 1.36 秒、释放等待 40 ms，
Node 实际运行后以 `0xc0000142` 退出，等待 95 ms、清理 52 ms。完整 V2 同
PID/TID 明确记录 kernelbase.dll 在 DLL_PROCESS_ATTACH 初始化失败，继而
kernel32.dll 子系统加载返回 `0xc0000142`；CDB、Job 与 raw 清理全部确认。
这定位了失败模块，尚未识别导致初始化失败的具体系统操作。

下一次仅做固定测试诊断 A-B-A：同一临时 profile/task/home、缓存 Node、固定打印
样例与环境，只比较 runtime capability 和额外 registryRead。候选依据是
[Chromium 共同添加该能力](https://chromium.googlesource.com/chromium/src/+/lkgr/sandbox/policy/win/sandbox_win.cc)
及[其注册表读说明](https://chromium.googlesource.com/chromium/src/+/lkgr/docs/design/sandbox.md)，
不能把 Chromium 的 broker/启动方式当成本项目的安全或因果证明。

A1 无 CDB 且完整验证后仍为 0142 才派 B；每组 fresh Job、精确 SID 集合、四项
LPAC AccessCheck、原 mitigation 与 Resume1 均须通过并确认 stop/wait/drain。
A2 须再次 0142，B 只有 exit0 和精确固定 marker、运行时字节稳定且全部清理
通过才记 supports-candidate；否则 inconclusive。两成员仅测试构造器可生成，
发布构造器与 token 校验仍固定 runtime 单成员。原用例保留失败、不开 admission，
不执行用户/模型代码，不导出 PID、SID、路径或输出正文。将来采用前仍须私有
注册表及完整隔离负例、默认功能门禁和 Win10/11 客户端验收；当前没有产品授权
能力变化或 A-B-A 原生结果。

运行 [44027445443](https://github.com/ofoxai/ofox-desktop/actions/runs/44027445443)
（`97a91cb7`）原生 control 4/4、ACL 1/1，原 CDB 完整 V2 与 KernelBase 初始化
失败继续复现。无调试器 A1/B/A2 的 capability 数为 1/2/1，token/LPAC/IL/策略、
Resume1、运行时 hash、stop/wait/drain 与全部清理均通过，但三个 exit 都为
`0xc0000142`、零 stdout/stderr。registryRead 单独不足，产品权限不变；候选
比较改为显式 opt-in，默认不重复。

下一轮沿已验证链路观察 14 个固定 Nt 入口和返回：实际 PDB 布局、线程映射和
`bl /L` 安装先核验，持久 owned 返回断点匹配同 TID、RSP+8 和返回地址后才记录
EAX，并清除自身。访问掩码只读 EDX；端口/值名接口不套用文件参数。名字有界且
只在私有目录，收据只含闭集别名。完整原 Host Resume1 仍为归因前置，缺失配对、
嵌套或超限保持 inconclusive；实际失败操作与 DLL 失败分别记事实，不自动因果。
本地合成/实际 PowerShell 协议测试不能替代待执行的 Windows 原生验证。

运行 [44034826154](https://github.com/ofoxai/ofox-desktop/actions/runs/44034826154)
（`ef5fef70`）control 4/4、ACL 1/1、原实际 token/LPAC 矩阵通过；新增观察器在
`operation_layout_unverified` 拒绝，尚无 CF 尾确认、continue 或原 Host Resume。
因此没有系统调用结果，也不能用于解释原始 0142；调试器、raw、task/profile
清理确认。此前基线调试链原生通过的事实保持，新增类型求值依赖尚未成立。

修正仅针对诊断：移除新增的 PDB sizeof/空指针字段取址依赖，按
[公开结构](https://learn.microsoft.com/en-us/windows/win32/api/ntdef/ns-ntdef-_object_attributes)
与 Windows x64 ABI 使用固定偏移，先对同一实际进程句柄取得
[IsWow64Process2](https://learn.microsoft.com/en-us/windows/win32/api/wow64apiset/nf-wow64apiset-iswow64process2)
成功的 UNKNOWN/AMD64 组合，并核对原生绑定 size/offset。每次 OA.Length 和可读
范围再核对；可选名字未知不阻断真实状态配对。原符号/gflag/断点安装及完整
V2/HostResume 门禁保留。另修正本轮发现的 MASM/C++ 逻辑运算符混用，显式使用
MASM and/or。回归先 RED 后 GREEN；这些修正的原生运行尚待执行，不作 0142 修复。

接手复核（本地，`ef5fef70` 之上，尚未取得新原生结果）：

- 编译阻塞：临时 Windows 检查工程的特性列表与正式 Cargo.toml 一致，二者都缺
  `Wdk_Foundation`，所以这是正式代码在 Windows test 构建中的真实错误。链接
  工作区源码的 Windows 目标镜像先复现 E0432，仅出现在 lib test（`cdb.rs` 只在
  test cfg 编译）。修正为 Windows dev-dependency 的同一 pinned `windows`
  0.62.2 追加该特性；`cargo tree` 核对发布依赖边不含它，Cargo.lock 不变。
  修正后镜像 lib 与 lib test 的 Clippy `-D warnings` 通过，属交叉检查。
- 已有产物再核对：44034826154 的 promptLines=10、markerMentionLines=12，
  恰为 `-cf` 前十条命令各带 `0:000>` 回显（六条标记命令的回显加输出）；
  LAYOUT 行确有输出但数值不符，与已移除的类型求值依赖一致，不另作推断。
  据此，断点区段紧接的 `bl /L 10-23` 回显会被旧解析当作非法行，下次原生
  运行将必然在 `operation_install_unverified` 拒绝。现跳过提示符回显，带提示符
  的伪断点行仍不计数，14 行、启用、地址和线程要求不变。
- 微软 [bl 示例](https://learn.microsoft.com/en-us/windows-hardware/drivers/debugger/getting-started-with-windbg)
  显示当前输出为 `e Disable Clear  <地址> ... 0:****`。入口与返回断点解析均接受
  这段 DML 文字和线程列 `~` 前缀；线程限定列的确切形态没有文档示例，待原生确认。
- 失败观测原超过 32 条即整体 inconclusive，正常启动的注册表缺失可能借此
  掩盖最后的失败操作；现与调试器 256 次调用上限一致并全部保留。

回归均先 RED 后 GREEN：portable PowerShell 7.7.0、Node 24.21.0 下 CDB 相关
九个测试文件 69/69、0 skip。夹具改用生产形态的 `ConvertFrom-Json` 收据
（Int64），浮点、字符串、WOW64 与缺失均拒绝，删去 Int64 分支的变异会失败；
另覆盖 operation 阶段白名单、256/257 调用边界和非属性接口的 `root=2` 拒绝。
`cdb_protocol` 独立单测 8/8；独立审查未发现严重或重要问题。以上都不是 Windows
原生证据，不改变 token、capability、ACL、mitigation、超时或 admission，
不作 0142 修复。

运行 [44049635186](https://github.com/ofoxai/ofox-desktop/actions/runs/44049635186)
（`10bc4f72`）首次通过新增观察器的全部安装门禁：architecture/ABI、实际线程
匹配、14 行已解析且启用的线程限定断点均为 true；完整 V2、flags `0→2`、
pre-attach 与 final hold `1/2`、原 Host Resume1 成立。control 4/4、ACL 1/1，
Node 在 Resume 后 154 ms 以 `0xc0000142` 退出，未超时、未被 Job 强杀；
KernelBase DLL_PROCESS_ATTACH 失败及 kernel32 子系统加载 0142 再次复现，
CDB/raw/task/profile 清理确认，没有任何 `Win32 error` 文案。

但操作投影为 `completedCalls=0`、inconclusive，尚无失败操作或返回码。形态
计数给出原因：本次与 44020847288 都恰好比 `-cf` 命令多一条提示符行（34/33、
11/10）。CDB 在 `-cf` 后打印不换行的 `0:000> ` 等待 `g`，`g` 之后的第一行
输出接在其后；加载器最早的 IFEO `NtOpenKey(Ex)` 早于任何 loader snaps，因此
首条 ENTER 很可能被粘在提示符后而漏读，序号停在 1，之后每条调用都被拒绝。
stdout 比 44020847288 多 137 行，扣除安装区段约 90 行属继续后的观察输出，
说明入口断点确实命中；是否还存在返回断点等第二个问题，现有产物无法区分。

修正只在投影：主循环中提示符后紧接 `OFOX_NT_RAW_V1 ` 时去掉该提示符，计数
`promptJoined`；回显的命令仍不能成为标记，bl 列表内不剥离且带提示符的行仍不
计数；多重提示符或其他大小写的残留标记一律拒绝。独立审查以 20 万条随机变异
日志对比，除单个粘连标记外未接受基线拒绝的输入，观测结果无差异。
另增闭集 `firstRejection`（enter-order/identity/arguments、name、listing、
return、invalid-*、marker-unknown、unterminated、no-calls），不含原文。
RED 复现与原生相同的整体拒绝，修正后相关九个文件 71/71、0 skip。下一次定点
运行只回答：去掉粘连后能否配对出失败操作及 NTSTATUS；若仍不完整，由
`firstRejection` 和 `promptJoined` 指出下一处断点，不改隔离、权限或超时。

运行 [44055154306](https://github.com/ofoxai/ofox-desktop/actions/runs/44055154306)
（`9ea2b34e`）首次取得原生系统调用配对：安装门禁、完整 V2、hold `1/2` 和原
Host Resume1 均成立；`promptJoined=1` 证实首条 ENTER 粘在等待提示符后，
`firstRejection` 为空，固定集合覆盖完整，14 次调用全部配对。三项失败依次为：
NtOpenKey `0xc0000022`（GENERIC_READ）、NtOpenKey `0xc0000022`
（KEY_QUERY_VALUE）、NtOpenKey `0xc0000034`（KEY_QUERY_VALUE），对象均只能
归为 `machine-registry`；其后 11 次被观察调用均成功，Node 随后仍以
KernelBase 初始化失败、0142 退出。control 4/4、ACL 1/1，清理全部确认。

这是第一份失败操作证据，但不足以归因：被拒绝的具体 HKLM 键、成功调用的顺序
及 KernelBase 初始化期间最后完成的操作都没有输出。registryRead 的 A-B-A 已证明
单独不足，因此不据此调整 capability 或 ACL。

下一步仅补可见性，不改观察范围或任何隔离条件：报告新增按序的全部配对调用
`calls`（含成功），对象名按固定公开 Windows 词表投影为形态：首个词表外组件
（SID、盘符、镜像名等）之后全部为 `?`，纯数字只遮蔽自身，仍属闭集，不上传原文。
独立隐私审查以 18 个对抗性名字核对无原文外泄，并据其意见收紧遮蔽与 ASCII 匹配。下一次定点运行只回答：两次
ACCESS_DENIED 的 HKLM 键是什么，以及 KernelBase 初始化失败前最后完成的被观察
操作是哪些；若最后操作只是映射 KernelBase，则说明其 DllMain 的失败点不在当前
14 个 API 内，需要另行扩大观察集合。相关九个文件 74/74、0 skip。

运行 [44058284863](https://github.com/ofoxai/ofox-desktop/actions/runs/44058284863)
（`7faf49e6`）再次完整配对 14 次调用，`firstRejection` 为空，按序为：
`\Registry\Machine\SYSTEM\CurrentControlSet\Control\Nls\CodePage`（GENERIC_READ）
及 `…\Control\Session Manager`（KEY_QUERY_VALUE）的 NtOpenKey 均 `0xc0000022`；
`…\Session Manager\Segment Heap` 为 `0xc0000034`；随后 `\KnownDlls`、程序目录、
kernel32/kernelbase 的 section 打开与映射、`\Sessions\?\Windows\?` section、
NtConnectPort 及映射均成功；最后两项为 `\Device\ConDrv\Server` 与相对的
`\Reference` 的 NtCreateFile，均成功。此后 KernelBase DLL_PROCESS_ATTACH 失败。

两次注册表拒绝发生在 ntdll 早期且加载随即正常推进，不按原因处理。最后的被观察
操作是为新控制台创建 ConDrv Server/Reference。Node 以 `CREATE_NO_WINDOW` 创建，
需要新控制台；自 Windows 8 起控制台宿主 conhost.exe 由客户端在初始化期间作为
自身子进程启动，而沙箱以 `PROC_THREAD_ATTRIBUTE_CHILD_PROCESS_POLICY` 限制子进程。
当前推断：KernelBase 的控制台初始化无法创建 conhost 而失败。该创建调用不在
14 个观察 API 内，因此尚属推断，未直接观测；产品创建标志暂不修改。

为直接观测该推断，观察集合增加第 15 个固定入口 NtCreateUserProcess，按状态-only
记录（不解释参数、不上传映像名）；断点范围、安装行数、`INSTALLED count` 与
上传状态的行数上界都改为由同一定义表推导，范围外 ID 拒绝。下一次定点运行只回答：
在 ConDrv Server/Reference 之后是否出现进程创建及其 NTSTATUS（预期子进程被
策略阻止时为 `0xc000049d`）。若成立，再针对性修复启动标志；若未出现，则失败点在
控制台初始化的其他步骤，继续缩小。相关九个文件 74/74、0 skip。

运行 [44062635550](https://github.com/ofoxai/ofox-desktop/actions/runs/44062635550)
（`fcb87187`）15 个入口断点全部安装核验通过，覆盖完整、`firstRejection` 为空；
配对的 14 次调用与 44058284863 完全一致，ConDrv `\Reference` 之后**没有**任何
NtCreateUserProcess 调用，KernelBase 随即初始化失败、0142 退出。因此“conhost
创建被子进程策略阻止”的具体推断被否定：失败点仍在创建 ConDrv 句柄之后、任何
进程创建之前，具体操作不在当前 15 个观察 API 内。启动标志与隔离条件均未修改；
下一步需在扩大观察或无调试器的控制台模式对照之间选择。

按用户选择，下一次定点运行做无调试器的控制台模式 A-B-A：A1/A2 与产品相同使用
`CREATE_NO_WINDOW`，B 只改为 `DETACHED_PROCESS`（不分配控制台，因而不初始化
ConDrv/conhost），capability、LPAC/token、子进程与映像策略、Job、输入和 profile
均相同；A1 不能重现 0142 则不派 B。只有 A1/A2 均 0142、B exit0 且精确 marker、
全部隔离证明与清理通过才记 supports-candidate，否则 inconclusive。`Detached`
仅在 test cfg 存在，产品启动标志未改；开发工作流显式设置
`OFOX_DOCUMENT_STARTUP_COMPARISON=detachedConsole`，并有测试固定该拼写及所在步骤。
回执记录传给 CreateProcessW 的实际标志，A1/A2 须为 `0x08080404`、B 须为
`0x0008040c`，Windows 单测将其绑定到 windows crate 常量；独立审查未发现严重问题。
协议单测 5/5、相关 Node 九个文件 74/74 与工作流测试 19/19、0 skip；Windows 目标
lib/test Clippy 交叉检查通过。原 CDB 观察与 registryRead 候选行为不变。
