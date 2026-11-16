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
