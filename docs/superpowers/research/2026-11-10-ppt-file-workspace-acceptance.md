# PPT 文件工作区、规划与视觉增强实施验收

本记录覆盖 2026-11-10 开始的实际实施。当前状态：**本轮两项明确阻断修复的私有签名候选版已就绪，v8b 最终工程门禁及限定 Native 预览回归通过；一般视觉与叙事改进转入 Fizzy #1615，不再作为本轮候选交付的阻断项。真实 Native 人工辅助修订、受控多图嵌入及限定 WPS 往返有功能正例，历史自主原稿的 QUALITY FAIL 和整体自主质量未通过状态保留。没有宣称全面视觉、PowerPoint/Windows 或整个 PPT 主题验收完成，todolist 与总卡不关闭。** 用户收紧范围后的本轮没有新增模型/生图调用。本文不以测试、目录数量或固定样张代替真实质量验收。

最新预算检查点：总授权仍为 **US$4**，保守占用含历史未知预留 **US$3.136741**，未预留余额 **US$0.863259**，无活动预留，实际累计生图 **3 张**。用户已撤销此前全局最多 1 张的限制，后续仍须冻结每轮图片数量/USD 上限并核对剩余额度；这不是无限费用授权。实际账单未核，历史费用检查点不追改。

## 本轮阻断修复候选版与后续范围

用户明确要求先修复确定阻断、交付一版并列出缺陷；全面视觉优化继续在 [Fizzy #1615：智能创作工作流与文件预览优化](https://app.fizzy.do/6179973/cards/1615) 跟踪。该卡已定位，仍为未关闭、10 个步骤均未勾选；本地 roadmap 未改。本轮候选交付与整项任务完成分别记录，不要求先消除下表所有一般视觉问题。

| 本轮明确阻断及修复证据                                                                                                                                                                             | #1615 后续一般视觉与叙事改进                                                                 |
| -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| 蓝图/Architectural Plan cover helper 的 metadata 与自身 Panel 页码固定相撞。原生框回归先失败，随后选择避开页码的最大横向区域；左右交替面板及其他风格回归共 12/12 通过，独立复核通过。              | 模型自画图片与标题的分区、层级及字形遮挡；本轮 helper 修复不自动移动模型图片，也不修写旧稿。 |
| 原稿第 4 页原生箭头及 SVG marker 正确，但 JPEG 丢失方向头。限定三角 marker 规范化覆盖持久化预览、右侧按需页预览、模型 thumbnail/overview 三条路径；真实像素 RED→GREEN、相关 64/64 及独立复核通过。 | 卡片留白、构图丰富度、计划与最终叙事一致、未批准功能须标为拟议；原稿中的这些缺陷继续保留。   |

见[页脚修复回执](../../../.real-eval/ppt-completion-2026-11-10/v8b-cover-metadata/focused-receipt.json)、[箭头修复回执](../../../.real-eval/ppt-completion-2026-11-10/v8b-preview-arrowheads/change-receipt.json)及[三路径真实原页证明](../../../.real-eval/ppt-completion-2026-11-10/v8b-preview-arrowheads/actual-three-path-proof.json)。箭头支持范围为经过边界校验的普通直线及限定三角 marker，不保证任意 SVG 路径、曲线、裁切或继承样式。原始 PPTX、原 SVG 和旧生成缓存均未改写；旧 v8 原稿 QUALITY FAIL 不因预览修复而追改。另准备的付费视觉修订已[取消且未发送](../../../.real-eval/ppt-completion-2026-11-10/native-product-v8b-autonomous-repair-preparation/preparation.json)，0 API、0 预留，预算不变。

[v8b 最终检查汇总](../../../.real-eval/ppt-completion-2026-11-10/v8b-final-delivery-check-summary.json) SHA `05c44958e1a99889c31a4e2ba29004c0a3080e50fd6bd60a579da76067143c1e` 对应 [1960 项源码冻结](../../../.real-eval/ppt-completion-2026-11-10/v8b-final-source-freeze.json)，SHA `690e3953ac9fa8f76a414e9520cee9df73797d1b16174e7e29ef40ed824d630d`，最终检查后源码保持不变：

| 最终检查 | 实际结果                                                                                                                                                                                                                                                                                                                                       |
| -------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Runtime  | 724 通过、0 失败、0 跳过。                                                                                                                                                                                                                                                                                                                     |
| Frontend | typecheck、format 均 exit 0；264 文件通过、1 文件跳过，3323 测试通过、1 跳过、0 失败。                                                                                                                                                                                                                                                         |
| Rust     | fmt、all-targets Clippy 均 exit 0。首轮完整执行在库测试得到 4275 通过、1 失败、25 ignored，既有 pnpm 缓存管理器探测 timeout 原始日志保留；同一测试隔离 1/1 通过（3.30s）。同源完整缓存复跑最终 4514 通过、0 失败、27 ignored、22 个汇总，exit 0、441.458s，编译缓存命中部分 1.65s。未改 Rust、测试或 timeout，不将负载解释为已证明的唯一原因。 |

私有 `private-full-app-v8b/Ofox Desktop.app` 为 1.5.0、Team R65NVCLBMY，主 SHA `d61a59cab4876fe33374d41eaaa7e1485d1fbb01355a023905727f2f32bbad34`。[静态审计](../../../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v8b.json) SHA `6cf908ce86daed6375fea4efb338b88ef71a7d1a82ab04628c6df8f6277946aa` 核对 9 个 Mach-O、深严签名、Team/DR/entitlements、源码→staged→bundle 资源及嵌入 renderer；无发现。相对 v8 仅更新四个生产 runtime 文件及两测试、一夹具，Rust/renderer 没有重新 release 编译，主程序签名前代码前缀与 v8 相同；资源更新后正常 Developer ID 重签，不冒称全新 Rust release 编译。

根代理实际启动该私有候选（PID 43355），在未改的真实原稿 `1d9a…10357` 第 4 页看到三条向右箭头，展开后恢复左聊天/右文件，输入框为空且未发送。见 [Native UI 回执](../../../.real-eval/ppt-completion-2026-11-10/v8b-native-preview-ui-receipt.json) SHA `1df905a80da47d84406c3cea7bba1c60ea736da28d18edde9d47c20787e3ae71`；该项仅验证真实第 4 页预览回归，不是新自主生成，也不是整稿重新质量通过。页脚 helper 的本轮证明为原生框回归及包内源码身份，没有额外付费生成来代证。

交付物为 [macOS arm64 私有候选 ZIP](../../../.real-eval/ppt-completion-2026-11-10/delivery/Ofox-Desktop-PPT-candidate-2026-11-11-macOS-arm64.zip)，**180814658 字节**，SHA `4f7c30455dfa65af16bd56b6342dfa6f1b4750ffb1acb9d561ef27d4af88d3db`。CRC 通过；[自有临时目录解包核验](../../../.real-eval/ppt-completion-2026-11-10/delivery/archive-unpack-signature-receipt.json)再次确认深严签名 exit 0、主 SHA 与候选相同，随后仅清理该临时目录。未安装/替换 `/Applications`、未公证、未发布，安装版 `810f…57a93` 未变。根代理已唯一追加并回读校验 [Fizzy 范围/剩余缺陷评论](https://app.fizzy.do/6179973/cards/1615/comments/03h7s7onv5syxpcgiq67ms7oe)，见[回执](../../../.real-eval/ppt-completion-2026-11-10/fizzy-1615-scoped-delivery-receipt.json)；未覆盖原描述，卡仍 open，10 个步骤仍全部未勾选。

## 工作区与安装边界

只读确认后复用 `$HOME/code/ofox-desktop-1.5.2`，分支 `feat/assistant-1.5.1`、基线 HEAD `2ebe649158fe6cbacb81cf740ea5ca8ada3ddc17`，保留已有未提交改动。`$HOME/code/ofox-desktop` 为不同 checkout（`fix/codex-mcp-type`、`38a681f5f04d96ac480230af4c5d92467c4a1f7b`），本轮没有复制覆盖或撤销。`agent-refs` 下各上游单独核实。

安装版 `/Applications/Ofox Desktop.app` 为 1.5.0、`ai.ofox.desktop`、Team R65NVCLBMY；初查主程序 SHA-256 为 `810f6031cc226e235234b9dd321d8cb9714c4d09393e79724ef12d74e1d57a93`，与此前安装收据对应。本轮没有替换或重新签名安装版；私有验收包的正常构建签名另见后文，也没有发布。macOS 26.5.1 (25F80)、arm64；只读版本核对 WPS 12.1.25895、PowerPoint 16.109.1。版本存在不等同编辑验收。

## 已实施的行为

- 左聊天、右文件工作区复用同一会话；附件/生成结果卡片打开受权 PPTX/HTML，保留聊天 DOM、滚动、草稿和运行。支持宽度调整、键盘分隔条、收起、全屏、切换及窄容器聊天/文件切换，没有新增一级导航。
- PPTX 按 OPC 根 officeDocument 关系找到实际主 part 和有序 slide IDs，返回真实页数。按需渲染，每次只处理选中页，缩放、适应窗口、缩略图、跳页和上下页不调用模型。当前最多渲染第 1000 页；更大的真实总数与未渲染范围分别显示。旧持久化预览每批仍最多 60 张 JPEG，页号可到 1000，受权读取及备份同步。
- PPTX 主程序、OfficeCLI 和像素 worker 分离，保留原生 OMML，预览副本只为选中实际页选择已有数学兼容回退。此回退图不作为编辑证明。
- HTML 通过固定 loopback 端口、随机 token lease 提供；不使用应用同源文件协议。iframe 只有 allow-scripts、没有 allow-same-origin；子响应禁止网络连接、框架、对象、表单和任意外部依赖。父 frame-src 只允许 about: 与专用端口，禁止文稿导航到应用 origin。嵌入资源可使用 data URL；外部依赖阻断并报告缺失，脚本异常明确显示。没有放宽主应用 script-src 或暴露 IPC。
- 文件身份由 owner、generation、session、attachment ID、完整文件 SHA 共同约束；读取、子进程返回及 HTTP 发送前复验。异步迟到结果不能覆盖另一份文件。显式引用当前文件/页只加入草稿；点击页面或翻页不发送消息。
- 自动规划依次完成需求结构化、公开启用/许可/能力硬过滤、模型设计判断、冻结风格及逐页计划。用途、听众、语言、时长、密度、公式/图表/图片、品牌与可编辑性进入规则；manual 优先，理由可见。计划、视觉预算与引用随发送/队列/重试保留，不受之后输入框修改影响。旧客户端不带工作流标记时保持兼容。
- 每页计划记录主张、证据、关系、构图、密度、可编辑对象与图片用途。默认视觉增强关闭且零预算；可选自动/优先插画、图片数量及 USD 上限；无可靠单价时不假定免费。显式仅数量模式说明没有 USD 硬上限。允许整稿零生图。
- 生图沿用已有 region-aware 模型目录和 OFox URL builders，通过受控素材 ID 回填；不会把 base64 或磁盘路径交给模型。单素材最多两次可放行提交，已计费或不确定结果保留预算；已知未计费释放预算也不清除尝试次数。展示真实回执与缺失费用，不伪造百分比或完成状态。
- 原稿单页修订使用受权版本快照，只允许选中页；源模式不自动修复整稿。输出后核对主文档、有效 ContentType、页序、未选页面及其共享依赖；导航链接可保留并指向已授权修改的页。失败拒绝交付。新文件保存父版本和目标页，撤销切回受核对的旧文件，不删除版本、不重新生成。

## 资源完整性与场景

[来源覆盖审计](2026-11-10-ppt-source-coverage-audit.md)及机器清单逐文件记录固定版本、许可、实际页、版式和原生适配差异。五套完整公开文稿分别为 Blue Professional 10 页、Cartesian 10 页、Course Module 7 页、Product Launch 8 页、Architectural Plan 6 页，共 41/41 源页预览；Navy 原始 PPTX 为 0 个内容页、3 个布局、1 个母版。另有 19 项未开放候选。

五套外来来源的原生适配仍均为 `partial`，机器清单中的 `originalPagesIndividuallyMapped` 各为 0；Navy 是内置母版，不计作外来完整文稿。八页统一样张均标为**自编原生风格示例**；原始模板、部分原生适配、自编示例、实际用户文件分别展示。架构原稿真实裁切、字体替代和 Cartesian 固定 Chart.js 依赖均保留说明。原模板全页可达不等同所有构图已完整转成可编辑 PPTX。闭源 ofox-agent 未作为资产来源。

[离线文件链验收](2026-11-10-ppt-offline-pipeline-acceptance.md)记录学术/技术、模拟经营、虚构产品三个独立叙事案例（10+8+8 页），原生公式/图表/工作簿结构、所有渲染、数据口径、哈希、准确耗时和零模型/生图调用。产品采用自编素材，未冒充真实生图。70 页夹具单独计数。

## 检查证据及状态

任务证据根目录为 `.real-eval/ppt-completion-2026-11-10/`（忽略的私有目录）。

下表保留最初实施阶段的门禁及证据；后续 v2/v3 历史及 v4 最终离线门禁见后文，不把不同轮次的计数混为一次运行。

| 检查           | 状态及证据边界                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 前端全量       | 3265 通过、1 跳过；`frontend-full.log`。新增 HTML 策略契约另 27/27 通过，非原生 WebView 攻击证明。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| TS / 格式      | typecheck、format:check 通过，`typecheck-final.log`、`format-final.log`。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| 有意义 RED     | OPC/预算尝试、页号70持久化/受权读取/备份、未选页MC namespace绑定、诊断私有素材屏蔽、跨run同callId费用累计及私有输入错误均有先失败证据。                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| Rust           | `cargo test` 共 22 个结果汇总：4481 通过、0 失败、26 ignored（含未授权真实请求和平台条件用例）。库本身 4252 通过、25 ignored。`clippy --all-targets -- -D warnings`、rustfmt 通过；三项 lint 风格问题修正后另做对应聚焦复核。                                                                                                                                                                                                                                                                                                                                                                                             |
| 真实 Host 长稿 | 先抽页1/61/70与两倍缩放10.54s通过。后逐页按需渲染全部1–70页并另渲染70页scale2，共71张、70个不同页hash，164.48s，源SHA未变，key/model/provider调用0；`host-all70-preview.log`、`host-all70-4b430042-f47e-4e89-a327-98de620d0729/receipt.json`。                                                                                                                                                                                                                                                                                                                                                                            |
| 源稿Host下层   | 真实 `make_python` + OfficeCLI，70页原稿仅 `slide70.xml` 改变，预览首图为70.svg，源SHA不变；make_python13.744s、整项测试27.29s。`host-source-page70.log`。这是下层文件链，不冒充完整账户→模型→交付链。                                                                                                                                                                                                                                                                                                                                                                                                                    |
| Runtime        | 默认官方签名Node测试537通过/12失败，Canvas native binding不同TeamID；保留失败，不修改正式签名。相同22.23.4现有私有临时Node完整653通过/1失败（stdio夹具仍默认官方Node）；stdio显式私有Node后23/23通过。随后用同版私有Node及显式stdio/Python test覆盖完整重跑659/659通过、0skip；`runtime-private-final.log`。保留默认环境失败，不称默认生产签名门禁通过。Search102通过。                                                                                                                                                                                                                                                   |
| GUI 布局       | 900px明暗、键盘调整、页70跳转与引用、视觉设置；该70页UI夹具复用8张样图，**只证明布局**，不冒充真实70页后端。原模板UI另检查Cartesian第10页、架构第6页及真实问题提示。截图在 `ui/`。                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| WPS/PPT        | 初次因锁屏未执行；用户解锁后的2026-11-11补验：WPS12.1.25895中学术5页原生公式w→a保存/关闭/重开保留，仍有2个OMML对象；经营3页原生图表样式修改保存/关闭/重开保留，缓存值及2个内嵌工作簿仍在。数据值编辑命令未确认，不计数据编辑通过。PowerPoint16.109.1明确需要订阅才能编辑/保存，未验；缺字体提示仍在。`office-edit/receipt.json`及`ui/wps-*.jpg`。                                                                                                                                                                                                                                                                         |
| 原生WebView    | 2026-11-11私有keyless WKWebView harness（900×640）真实PPTX页70及哨兵可见；HTML计数0→1、输入echo可用。父DOM/storage、opaque storage、file/network、popup/nested frame被阻断，nativeInvoke/raw JS wrapper不暴露；进一步发现底层`window.webkit.messageHandlers.ipc`实际存在，直接发送缺少key和错误key的dummy消息后主probe仍1。没有把wrapper缺失当作底层入口不存在。子文稿导航应用origin后仍保留原lease URL；同大小文件替换后旧lease从200变404，重载显示新SHA。主框架正向probe始终1，key/model/provider为0。关闭文件后聊天引用保留。`native-ui-acceptance.json`及`ui/native-*.jpg`；没有替换安装版，不外推Windows或任意攻击。 |

## 当前真实模型基线与未完成验收

2026-11-11 用户要求直接使用 Desktop，已从已安装1.5.0的真实 UI、现有账户生成6页 Cartesian Transformer PPTX；没有导出密钥、复制凭据或替换安装版。该旧安装版不含本轮独立规划与右侧文件工作区实现，因此是**当前 Desktop 基线**，不能代替新源码的完整流程验收。模型改为经官方公开目录核价的 GPT-5 Mini，关闭联网，手动命名避免自动标题，实际图片/视频/语音/网页/MCP调用均为0。

真实运行先遇到 Python 括号错误、LaTeX 转义/子集不支持，随后两次文档预览成功。最终一条模型请求发生 `ECONNRESET`、无完整usage，触发 `assistant_cost_unavailable`，并非前五条请求缺报价；本轮不能记为完整交付通过。该安装版错误文案把它描述成“缺少可靠价格、继续本机配置”有误，已在工作区zh/en/ja改为中性的费用信息不完整、暂停并先查看请求记录；三语 UI 文案回归不扩称后端各故障原因的端到端测试。

实际原件 `Documents/Ofox-PPT-真实验收-20261111/Transformer_6min_cartesian-GPT5Mini.pptx` 为128272字节、真实6页、SHA `edd0f3da0f1d35f503205d1a8a25e7e484a932fb3bba9ba2cc82002c460786fd`。已在应用中逐页浏览1–6，确认5个原生OMML及5个规范公式PNG回退。三条第2页遮挡警告经实际图像确认；另发现字面LaTeX、方差表述及多头连线等问题，不能用“已生成”代替质量通过。

OfficeCLI在副本中修正上述问题，保留原件：`Transformer_6min_cartesian-核验修订版.pptx`，129750字节、SHA `ce52f04f8fc551ea84144c287fb98b5be9a1bc45f4ed797924d911fdf60e03f6`。机械问题3→0、schema0，六页渲染并独立目检；五个核心MathChoice/Fallback语义及回退图保留。随后在WPS12.1.25895逐页核对六页，确认原生公式正常显示、四条汇入箭头清楚，关闭后文件字节未变；缺失字体提示仍在。另在原件的私有测试副本中对第4页原生注意力公式加入等价的一元正号，保存、关闭、重开可见，文件内仍为OMML分数结构（共5个OMML）；不是将图像或普通文本改字当作数学结构编辑。证据：`installed-ui-repair/repair-receipt.json`、`office-edit/direct-gpt5mini-wps-receipt.json`、`office-edit/checked-wps-visual-receipt.json`及`ui/checked-wps-page*.png`。

费用不再记为全任务0：五条完整请求SDK估算合计US$0.05699928，实际账单字段均未核实；保守把全部input按最高输入价计、逐笔向上取整为US$0.058184。未知请求按最多3次POST、每POST US$0.129536预留US$0.388608，该基线结束时任务费用上界US$0.446792，US$3预算剩余US$2.553208。`native-desktop-cartesian-usage-audit-2026-11-11.json`逐笔记录；`native-desktop-shared-budget.json`用于后续发送前预留。旧独立入口已在任何凭据读取之前退役，不能另起一份US$3额度重复执行。

第一份私有构建前的前端门禁3277通过、1跳过（264个文件通过、1文件跳过），typecheck及格式检查通过。首轮发现Vitest错误收集忽略目录`.real-eval`中的六组独立Node测试，造成“无Vitest测试套件”；仅将此私有目录排除，保留单独Node回归记录，完整重跑通过。未删除产品测试。

第一份新源码完整release验收构建成功（Rust release 10m16s），仅输出到工作区及私有验收目录，不替换`/Applications`。已由应用UI备份30个对话（本机私有ZIP、CRC核对通过，应用声明不含API Key），并在没有编译进程时清理该checkout已完成的dev编译输出，保留release缓存、源码、测试证据及当前Host测试二进制副本。私有副本通过deep/strict签名复核；9个Mach-O均为原应用同一Team、hardened runtime，runtime资源与staged逐字节相符。正常退出旧版后通过UI启动私有副本，实际进程路径与SHA `5906312636faddb7ebd4bf4f87a511f594d473176eed8a40e599215625f4f005`相符，原有历史可见，正式凭据没有导出。此包未公证、未安装、未发布。证据：`private-full-desktop-launch-receipt.json`、`private-full-desktop-static-audit-2026-11-11.json`及构建日志。

私有完整应用的文件工作区实测：自编70页真实PPTX可直接跳至70，画面可见唯一哨兵`PAGE_SENTINEL_070`，并可明确引用该页到草稿。切换到自编HTML后按钮计数0→1、文本回显正常，父DOM与来源存储访问被隔离；故意缺失的图片同时在文稿和宿主显示错误。展开、恢复分栏、关闭后，PPTX页引用与HTML文件引用及草稿均保留，未发送模型请求。原始模板也已在此应用逐页浏览：Cartesian10、蓝色专业10、课程模块7、产品发布8、架构蓝图6，共41页全部图像可达；字体替代、固定Chart.js及架构原稿裁切说明可见。这些是来源完整性与文件预览证据，不代表全部布局原生适配。见`native-full-app-original-pages-ui.json`及`ui/new-source-*`。

第一例新源码真实学术自动选型没有生成文件，不能记为通过。模型把原生编辑要求推断为`roundTrip=verifiedRequired`，6个候选因尚未认证而正确被排除；随后4次相同冻结与一次无计划生成均被拒绝。工具硬过滤未被绕过，但模型需求解释及错误引导不足。已补充原生编辑与预认证的明确语义、空候选的具体原因和停止/澄清指引，保持用户显式硬约束不变；24项规划回归通过并独立复核。后续 v2 确认候选和冻结可达，但出现下文记录的其它协议与质量问题。该例9条请求用量完整，保守逐笔取整US$0.053291，SDK估算US$0.03585621，账单未核。该步骤结束时累计任务费用上界为US$0.500083、预算剩余US$2.499917，生图仍为0。见`native-desktop-academic-auto-audit-2026-11-11.json`。

生成文件“引用当前页”初次操作没有可见响应，但在未修改代码的同一应用重新打开后，第4页引用正常进入草稿，故初次现象暂未稳定复现，不能声称已定位修复。普通附件的PPTX页引用及HTML文件引用均已实际成功。另独立复现了保存消息快照先到、历史尚未刷新时的PPTX/HTML引用失败：引用校验现与工作区按同一顺序合并消息、流和当前附件，保留身份、版本、类型及当前缺失状态门禁；2项真实RED后整份hook测试35/35通过。此修复不能倒推首次完成态无响应的原因。

## v2 真实复验与 v3 修复历史

本文的v2/v3/v4指本轮私有验收构建轮次，不是对外发布版本号。第二份私有完整包主程序 SHA 为 `ccfaf7fca6df936ec85433798d58da84e0eb22e1affac7584fc8c1de463b59d9`，release构建12m12s；实际进程路径和SHA已核对，安装版仍为原 `810f…57a93`。独立静态复核确认deep/strict及9个Mach-O签名、同Team和权限，全部runtime/skills资源与该包构建时冻结的源码一致；233个dist原始资源payload及全部234个资源key可在主程序定位，另一个index.html为Tauri处理版本，引用该次主chunk。新引用校验也在该嵌入chunk中，但不含之后的primer、冻结元组补全和fork修复。该包未公证、未安装、未发布。见`private-full-desktop-live-repair-launch-receipt.json`、`private-full-desktop-static-audit-v2-2026-11-11.json`。

原生完整应用窗口已缩至900px CSS宽（2倍屏截图1800px），实际检查浅色和深色主题；聊天/文件切换后草稿及第6页位置均保留。最后恢复原system主题、1000px窗口并清除测试草稿，未调用模型。该记录是真实完整应用的有界布局验收，区别于早期复用8张图的合成70页UI夹具，不外推三语、Windows或所有窄屏状态。见`native-full-app-900-receipt.json`及`ui/native-full-app-900-*.png`。

v2使用同一学术需求、GPT-5 Mini和Auto，候选筛选及Host冻结选中了Course Module。但模型反复遗漏/不匹配计划元组、遗漏format、未及时载入PPT技能，使用原始`Presentation()`触发风格保护，随后遇到不支持的LaTeX宏。记录共16次`create_document`工具尝试；根代理在UI约04:21处人工“插话”，明确要求保留计划、先加载技能并用`new_deck`/`equation`。两个run各18和7条模型请求，唯一`use_skill`在第19个工具调用才出现。因此最终交付不能记为无人干预Auto流程通过。

最终成品`Transformer_6min_course_module.pptx`为110460字节、实际6页，SHA `7c44bfefe2d3b7a46a877c26f416f9d3b682eddb9ec528d618c1f87ee0cc1231`，含6个OMML、6个公式回退、6页备注；schema检查0错误并不等于质量通过。六页兼容渲染及两名独立审查确认：第2页目标侧栏与首张卡片重叠；第4页辅助说明仍有损坏/字面LaTeX；第5页把并行head画成串行链；第6页连线会误导为FFN之后才做因果掩码；多页构图重复、留白过大。带3条警告的同一预览未修复即交付；该成品未做目标办公软件编辑验收。见`native-desktop-academic-auto-retest-audit-2026-11-11.json`、`native-auto-retest-2026-11-11/`。旧文件及失败证据保留，后续helper修复不会追溯修改它。

基于上述实测，源码在 v3 阶段补入以下措施；**这些措施的聚焦通过不等于真实模型已自行完成合格成品，当前 v4 终态结论仍待补录**：

- Host仅对明确PPT上下文且工具/runtime/PPTX能力可用、没有显式slash技能的每轮请求，预载未被用户同名技能覆盖的固定内置PPT正文。正文完整受限读取、名字及路径复核，读前后核对身份/取消；成功后才给运行时内部标记，不写用户技能选择、不制造工具活动、不增加授权。旧客户端可能再调一次`use_skill`，此兼容限制保留。见[primer实现说明](2026-11-11-ppt-builtin-primer.md)。
- 只使用本轮Host已确认的冻结计划填补遗漏的planId/revision；显式冲突、空值、陈旧或跨run元组仍拒绝。v1指导统一为需求、候选、合格ID、recipe、冻结再生成，不能通过重新冻结或换格式绕开约束。
- Python新增原生`fork_join`，连接共同输入到每个独立分支再汇总，不连成head串行链；支持可选端点。lesson正文越过主区只报告诊断，不静默挪动显式几何。diagram边界使用所属PPT真实尺寸，已补4:3越界拒绝及20×9宽页正例，未把默认16:9网格当作所有原稿尺寸。

WPS 12.1.25895针对独立自编一页fork/join样张已完成限定往返：原生箭头输入→四分支→汇总可见，修改私有副本一处分支文字后保存、关闭、重开保留；8条原生连接线仍在，原始样张SHA不变。没有移动连接线或测试所有对象，也不是新模型Auto成品验收。OfficeCLI JPEG有箭头glyph显示限制，与此次WPS原生显示分开。见`fork-join-proof/wps-roundtrip-receipt.json`及`ui/wps-fork-join-*.png`。

WPS图表数据编辑仍未通过：经营样稿及在WPS自身创建并保存的控制图表都未打开可编辑数据表；正常退出重启后控制例仍未确认数据单元格编辑，曾出现“正在选择图表数据源”的提示。包内结构核对发现有效内嵌XLSX和匹配缓存，因此不能据此确定生成器缺失工作簿，也未为该现象修改源码。图表样式往返通过与数据值编辑是两项不同结论；PowerPoint订阅限制、字体提示和Windows未验状态不变。见`office-edit/chart-data-probe-receipt.json`。

25条原生请求回执、25条assistant用量和应用请求列表逐项一致，均HTTP200、complete，保守全input（含cache）最高US$0.25/M加output US$2/M、逐笔向上取整为US$0.264988；SDK估算US$0.14380577，实际账单仍未知。显示的`modelRequests=26`额外计入了1个cost0的本地document generation，不是第26个provider请求；该统计口径尚未修改。该 v2 步骤结束时的共享账本上界为**US$0.765071**，当时 US$3 授权余额为**US$2.234929**，生图0/最多1张；此前ECONNRESET的未知预留未释放，新发送仍需先预留，独立旧入口继续退役。

v3 阶段历史检查快照：前端该轮全量以4 workers运行3282通过、1跳过，typecheck/format通过（`frontend-final-primer-fork-receipt.json`、`frontend-final-primer-fork-unit.log`）；此前并发高负载3个Library用例失败与模块级60秒fixture票据在67.632秒suite内过期吻合，隔离33/33通过，未改无关fixture。该阶段 Rust 22个结果合计4494通过、0失败、27 ignored（lib4260通过、25 ignored），rustfmt及all-targets Clippy通过。primer聚焦8/8、冻结元组相关94/94、技能示例12/12、diagram+既有style真实Pyodide14/14通过并独立复审。该阶段 runtime 在现有私有Node22、显式Python/stdio测试覆盖的隔离环境中运行，未豁免正式包签名门禁；首次674项中673通过、1失败，唯一失败是MCP进程退出观察；隔离安全测试1/1通过，随后相同源码串行全套**674/674通过、0失败/跳过/取消，129.551748s**。首轮失败及隔离记录保留，根因尚未确立，不记作确定的负载噪声；最终收据为`runtime-final-primer-fork-tuple-receipt.json`，日志为`runtime-final-primer-fork-tuple-serial.log`。早先662/662属于v2前一轮。最终v3 release构建已于11m32s完成，私有副本主SHA `31aee87e915f2c661cf8c8e757a88643b20e6b8c4b5db0631b5bf7c9e16cdda9`通过独立deep/strict与来源审计：9个Mach-O、source→staged→bundle资源、234资源key/233原始renderer payload及Rust primer标记均核对，安装版SHA未变，源码前后guard稳定。v2→v3实际改变document-tools、planning、deck.py及PPT技能4个runtime文本；收据`private-full-desktop-static-audit-v3-1794342778850.json`。此为私有构建与静态来源通过，不代替新包启动及无人工协议插话的真实模型复验。

## v4 已核事实与当前待验项（2026-11-11）

v4 私有包主程序 SHA-256 为 `6914a1e747c2a7808a966c0f316d18b0f09e573865b4a5ab1cd9872a17fa5550`，仍是应用标识 `ai.ofox.desktop`、包内版本 1.5.0，本轮序号不是发布版本。安装版 `810f…57a93` 未替换。独立只读审计通过 App deep/strict、9 个 Mach-O 的逐项 strict、Team R65NVCLBMY、hardened runtime、时间戳及权限；Node/OfficeCLI 只有既有 allow-jit，Canvas 无额外权限，DR 与安装版一致。源码→暂存→私有包的 34 个 runtime 文件、212 个技能文件及 Python/notice 资源逐字节相等；暂存 11854 个资源与包内 11855 个资源的唯一额外项为声明过的 `PI_LICENSE`。234 个前端资源键、233 个完整原始 payload 和 Tauri 处理后的入口 HTML 均有二进制定位证据，包含新紧凑布局、准备阶段及费用阈值；245 项 runtime 与 14 项前端冻结快照匹配。见 `private-full-desktop-static-audit-v4-1794347118812.json`。构建明确跳过公证；这不是 Gatekeeper、Windows 或模型成品质量认证。

最终离线门禁与历史轮次分别记录：Rust **4499 通过、0 失败、27 ignored、22 个结果组**，rustfmt 和 all-targets Clippy 通过；runtime 在指定私有 Node 22、精确隔离环境及两项测试 Node 覆盖下 **685/685 通过，0 失败/跳过/取消**；前端固定 4 workers 的唯一完整套件 **3314 通过、1 跳过、0 失败**，264 个测试文件通过、1 个跳过，typecheck/format 通过。前端门禁前后 14 个相关文件哈希一致；旧 673/674 的失败及后续恢复记录仍保留，不追改为首次全绿。收据：`final-v4-code-gates-receipt.json`、`runtime-final-v4-full-receipt.json`、`frontend-final-v4-compact-receipt.json`。

本次 v3→v4 的 5 个 runtime 文本改动已随包核对，新增边界如下：

- 规划 schema 和拒绝诊断明确总页数包含封面/结尾、源模式只数授权目标页；逐页 editing 是全局可编辑性策略的继承，不要求为通过校验添加无关对象，不降低原要求。
- 有界 LaTeX 子集新增转置、三种省略号、带花括号平方根/嵌套平方根及短 ASCII upright 名称，生成原生 OMML；未知宏、非法名称、未支持的根和过深嵌套仍在添加形状前拒绝。兼容 PNG 不作为原生编辑证明。
- 对真实 OfficeCLI `pat7` 图片填充的两位小数尺寸量化保留原精确比例判据，并只增加各边 ±0.005 的有界恢复；实际最终 JPEG 的公式区域已先出现 0 墨迹 RED 再通过。PNG/外链/namespace/plain-rect/祖先属性限制未放宽，原始 SVG 与交付 PPTX 不因预览而改写。
- 流式工具参数阶段有独立的“正在准备下一步操作”，真实执行开始后才显示执行类别；不会提前记执行/成功，不暴露参数或源码，不伪造百分比。“确认生成计划”与查询风格分开。取消、旧回调、会话和账户隔离有回归；新阶段不能倒推旧 v3 的泛化标题是漏发工具开始事件。见 `tool-preparing-focused-receipt.json` 及 `tool-preparing-copy-green.log`。

本轮新增可选文字费用**暂停阈值**，显式 `textMaxCostUsd=0.10` 仅冻结到本次发送、队列或分支；旧消息重试继承原策略，不自动套用新阈值。缺省保留原行为，显式 1 也可仅启用费用观察而不增加 local/MCP 权限。达到已记录费用阈值后停止后续请求，最后一个请求及重试仍可能额外收费，未知费用继续停止；这不是服务端费用硬上限。公开报价预留已单独核对完整输入/输出上限及最多三次 HTTP 尝试，没有假定 6000 输出。`v4-native-text-quote-receipt-2026-11-11.json` 是准入报价，不是本次实际账单或已消费费用。**正在进行的 v4 真实案例及最新共享账本终态由根代理后续补录，此处不预填成功或新增结算。**

真实 v4 原生窗口在 1000px 分栏中可见第 5 页，文件头、版本/兼容说明和工具栏已收紧；优化与换图在“更多”中，仍只准备草稿。900px 文件视图的第 6 页在浅色、深色及恢复 System 的存档截图中均完整可读，缩略图和页码保留。初步屏幕观察曾被解释为持久空白；两名代理独立逐图复核不支持该结论，现已撤回，瞬时观察与截图的差异尚未复现，未基于此修改源码或构建 v5。原 System 主题、1000px 窗口已恢复，测试草稿已清、文件工作区已关闭；该 UI 检查没有新增收费调用。`native-full-app-v4-ui-receipt.json` 记录这组有界观察，与复制时 `launched:false` 的历史收据分开；不外推三语、任意窗口或 Windows。

WPS 12.1.25895 对一页自编数学证明另完成真实内部编辑：原稿及重开副本均保留 5 个原生公式/5 个根结构，在第一处根的 `m:rad/m:e` 内加入 `+1` 后保存、关闭、重开仍在，原稿 SHA 不变。首次只改普通段落前缀的操作明确不计原生编辑通过；字体警告和编辑后左对齐重排仍保留。见 `math-commands-proof/wps-roundtrip-receipt.json` 及对应 before/native-edit/native-reopened 截图。此为单个自编公式的限定往返，不能认证新模型成品的全部公式、图表数据编辑、PowerPoint 或 Windows。

## 原生真实生成首轮终态与修复（2026-11-11）

以下案例均从完整 Desktop UI 普通发送，保留账户原生凭据使用方式；未再次读取或导出钥匙串。v4 三例均使用 GPT-5 Mini、关闭搜索，手动命名会话，未人工插入工具协议。它们与静态八页自编样张无关。学术六页内容固定；经营数据为自行编写的模拟数值；产品 Focus 为虚构概念，未假装真实发布。

| 案例             | 实际文件与流程                                                                                        | 已核结果与失败边界                                                                                                                                                                | 费用保守上界                                              |
| ---------------- | ----------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| v3 GLM 学术 Auto | 6 页、104319 字节、SHA `1b66b4ae…c24cb7d4`；22 完整请求、1 未知请求，1 次普通本地命令授权，无协议插话 | 6 个 OMML；多头误画串行、字面数学标签，旧 JPEG 漏一条已有公式；交付后连接错误，失败保留                                                                                           | US$0.841670，含未知请求 US$0.668469 预留                  |
| v4 学术 Auto     | 课程模块，6 页、63855 字节、SHA `88be10dd…aa7218`；13 完整请求，显式文字暂停阈值 US$0.10              | 并行 head 各自汇入正确；仅第 4 页有 2 个 OMML，第 3/5 页核心公式是普通文本；第 6 页显示对象字典，第 5 页有字面反斜杠换行。文件已自动保留，但终态 `assistant_cost_limit`、质量失败 | US$0.164670；SDK 估算 US$0.10243090，无未知请求           |
| v4 经营复盘 Auto | 蓝色专业，8 页、51550 字节、SHA `45b0a278…743471a`；10 完整请求，预览后显式交付，完成态               | 两个原生图表及两个内嵌工作簿，缓存/XLSX 均为给定数据；假设、部分成本与模拟口径正确。第 4 页图表与口径文字原生框重叠 0.25 英寸，WPS 同样可见；不能用 0 条自动发现声称视觉通过      | US$0.102397；SDK 估算 US$0.06463144，无未知请求           |
| v4 产品概念 Auto | 产品发布，8 页、39066 字节、SHA `53542a63…2051219`；10 完整请求，完成态                               | 最终仍有 9 条警告，第 4 页也出现字典正文；冻结了可选封面图片 ID，但未调用生图，明确回退原生视觉。预算摘要缺失使模型重复询问已给出的授权，属于待修复流程问题                       | US$0.118891；SDK 估算 US$0.08180103，无未知请求、图片费 0 |

证据分别在 `native-glm-v3-academic-auto-final/`、`native-gpt5mini-v4-academic-auto/`、`native-gpt5mini-v4-business-auto/`、`native-gpt5mini-v4-product-auto/`。其中逐笔 `budget-audit.json` 与内容/视觉审计分开；实际账单字段仍为 null，不能把 SDK 估算或保守全输入计价当作账单。计数采用真实 provider 回执，不采用包含本地文档操作的 `modelRequests`。在这四例及此前历史结算之后，共享账本保守占用 **US$1.992699**，US$3 授权未预留余额 **US$1.007301**，生图 **0/最多 1 张**；历史两条未知结果预留仍保留。这是本段记录时的快照，后续发送必须再预留，不能据此重置额度。

v4 经营与产品的实际文件八页已分别在右侧预览逐页到达，截图 `ui/v4-native-business-page1..8.png`、`ui/v4-native-product-page1..8.png`，绑定对应完整文件版本；学术实际第 5 页见 `ui/v4-native-academic-auto-page5.png`。可达性通过不代表图中文字与布局通过。学术的两个预览确有 JPEG image block 进入后续 vision SDK 上下文，不只是路径或文字发现；未抓取原始 HTTP 请求体，也不能由入模事实推断模型已正确理解每页。见其 `preview-vision-audit.json`。

对经营真实文件的私有副本，在 WPS 12.1.25895 中修改原生折线图样式、保存、关闭、重开后修改仍在，两个原生图表、8 页及原数据保留；原稿 SHA 未变。再次点击编辑数据仍未出现数据表，窗口菜单仅有本文件，**数据单元格编辑未通过**。该样式验收副本不是布局修订交付稿，样式也引入了轴标题占位，不能冒充自动质量修复。见 `office-edit/business-v4-wps-receipt.json`、`ui/wps-v4-business-*.png`。

针对实际失败，追加有意义回归后修复：拒绝把结构化对象隐式 `str()` 成可见正文，保留合法富文本和用户主动输入的字面文本；在保存前检查原生 chart 框与外部非空文本框交叠，限定目标页并保留有界诊断，不静默移动对象；费用提示三语去除写死的 1 美元及服务端硬上限暗示；Host 冻结结果提供由当前策略与实际预留派生的视觉预算快照，运行时不再把已授权误述为预算缺失。预算摘要不构成预留或能力，实际生图仍逐次核对版本、报价与原子额度。这些修复已完成独立源码审查、离线门禁和 v5 包内来源核对；新原生成品、真实修订及生图仍待实际验收，不能由摘要传递或构建成功提前判为通过。

原生数学结构检查的现有边界也已核实：仅 `visual.kind=native-math` 的计划页必须有真实 namespace 的 OMML；继承 `editing.formulas=true` 是条件编辑策略，不代表每页存在公式。因此学术第 4 页满足结构合同，不等于第 3/5 页实际核心公式全部原生化。此次没有用全页强制添加无关公式或自然语言/ASCII 正则来伪造覆盖。

产品原稿另在 WPS 12.1.25895 实际打开，第 2 页主标题完全不可见，副标题仍在；封面完整。截图 `ui/wps-v4-product-page2-title-hidden.png` 与同 SHA 的应用预览存在差异。源码及原生 XML 确认：`content_slide` 先复制标题占位符，再追加不透明页眉，更新标题文字时未修正层级；第 2–8 页均有相同 XML 结构，但此次原生截图只直接证明第 2 页。该旧原稿及失败证据保留，不因 OC 可读 JPEG 追改为通过。

标题层级修复已有真实保存 XML 的先失败后通过回归及独立审查：`_native_title` 将同一标题元素移到既有装饰之后、`p:extLst` 之前，保留原生占位符、ID 及其他对象相对顺序，不复制为图片或重建标题。相关聚焦 11/11 通过，涵盖六风格的封面、章节、正文及 legacy 继承边界。另以两页自编样例（SHA `11ded939aaf8e4903c7ac799f17a07ca093e7a4b06b3d20b3e4a78bc58d8081e`）在 WPS 12.1.25895 实际打开，第 2 页标题与副标题可见，原生标题占位符仍为 ID 2；未在 WPS 编辑或保存该样例，字节未变。见[限定显示证明](../../../.real-eval/ppt-completion-2026-11-10/title-layer-proof/wps-native-receipt.json)。它不是重新生成的产品案例，也不认证整稿编辑、往返或成品质量。

## 私有 v5 构建与离线门禁（2026-11-11）

最初流水线在 Clippy 的 `nonminimal_bool` 停止；其中已通过的 Rust 测试保留为历史，整条流水线没有标成成功。随后仅把等价谓词 `!is_some_and(cost > 0)` 改为 `is_none_or(cost <= 0)`，独立复核确认 `None`、正值、零/负值及 count-only 短路行为一致。旧 `v5-final-source-freeze.json`（SHA `005f88e3…fbb3a`）未覆盖；新的 1953 文件冻结为 `v5-final-source-freeze-after-clippy.json`（SHA `e3103ebb…7ff4cb`）。新流水线重新执行 fmt、all-targets Clippy、完整 Rust 测试，再正常构建、复制和深严核验，全部退出 0。见[v5 流水线收据](../../../.real-eval/ppt-completion-2026-11-10/v5-after-clippy-v5-private-check-build-pipeline.json)。

该冻结对应 Rust **4503 通过、27 ignored、0 失败**；运行时最终 **700/700 通过、0 跳过/取消/失败**，323 项源码/技能/测试原始记录前后一致；前端 **3323 通过、1 跳过、0 失败**，typecheck/format 通过，1083 项前端源码/测试/配置前后不变。运行时见[最终收据](../../../.real-eval/ppt-completion-2026-11-10/runtime-title-layer-final-receipt.json)，前端见[最终收据](../../../.real-eval/ppt-completion-2026-11-10/frontend-post-live-fix-receipt.json)。这些计数分别对应实际门禁，不与旧 v3/v4 计数相加，也不外推 Windows 或模型质量。

正常 release 构建生成的私有 `private-full-app-v5/Ofox Desktop.app` 主 SHA 为 `64aff399ba6d0859ce71a36de5b5cfe51f6dcb3612405e4b2eab09c24435a78c`，仍为 1.5.0、`ai.ofox.desktop`。独立[静态审计](../../../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v5-1794352951870.json)确认 9 个 Mach-O 的深严签名、Team、DR 和 entitlements，34 个 runtime 源文件与 212 个技能资源在源码→staged→bundle 中逐字节一致，整体资源仅多一个明确声明的 `PI_LICENSE`；234 个 renderer 资源 key、233 个原始 payload 及 Tauri 处理后的 HTML 均在主程序中核对。1953 项冻结、323 项运行时与 1083 项前端记录在审计时匹配，三语费用文案、typed guards、chart 诊断、可信预算摘要及标题同元素置顶标记均在包内。安装版 SHA `810f…57a93` 未变，没有替换或发布；未做 notarization/Gatekeeper 或 Windows 认证。静态通过不等于新的原生模型案例或所有标题/对象已通过验收。

v5 仍有明确的 Pro 单图 wire 兼容缺口：planned Host 在模型省略数量时也会内部写入 `n=1`，而精确模型 `volcengine/doubao-seedream-5.0-pro` 的已核接口约束不接受该字段。仅在提示中省略 `n` 不能避开 Host 注入；此前准入建议已由[更正收据](../../../.real-eval/ppt-completion-2026-11-10/seedream-image-response-admission-receipt.json)标为失效，不能据此在 v5 发起该 planned 收费图。四组回归实际 RED 为 2 失败、2 通过后，已应用窄修复，仅对该模型省略 wire `n`，保留 Host 单图计数与预算门禁；实际代码与草案已独立复核，未发现新增阻断。聚焦 [GREEN 4/4](../../../.real-eval/ppt-completion-2026-11-10/pro-wire-green.log) 已通过。v6 此轮 Rust 全量 **4507 通过、27 ignored、0 失败**（22 个结果组），fmt 与 all-targets Clippy 退出 0；正常私有 release 构建、复制及深严检查已完成。见[v6 流水线](../../../.real-eval/ppt-completion-2026-11-10/v6-private-check-build-pipeline.json)。v6 主 SHA `c5e6b633385253c7896c429cb387d430e7836e8d7c5d1f2afa729a72d158f51f` 的独立[静态审计](../../../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v6-1794354879039.json)通过 9 个 Mach-O 签名、1953 项 v6 冻结、源码→staged→bundle、323 项运行时/1083 项前端记录及嵌入 renderer 核对，Pro 专属编译标记存在，安装版未变。此时的真实生图与交付尚待执行，后续实际结果及新发现的鉴权 P1 另记下文；静态来源通过不等于安全或成品质量通过。该 Pro 修复不计入不可变的 v5。

## v5 学术 Auto 与手选对照终态（2026-11-11）

两例均由真实私有 v5 普通发送，模型为 `openai/gpt-5-mini`，同一提示 SHA `c2e67617…781e20`、文字暂停阈值 US$0.15，关闭网页搜索与生图；仅 Auto/显式 Cartesian 选择不同。保留原生出稿与独立质量审计的区别，不把一次对照推广为路由或模型质量的普遍结论。

| 案例                  | 真实流程与成品                                                                                                                                                                                 | 检查结论                                                                                                                                                                                                            | 费用记录                                                                                                                                                                   |
| --------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| v5 学术 Auto          | 自动选择 Course Module；6 页、117329 字节、SHA `bb4e06d1…ecfd4`。16 个完整 provider 回执、0 未知请求；多次 Python 失败后成功预览，下一次创建被 `assistant_cost_limit` 暂停，最后预览自动保留。 | 第 3/4/5 页共 7 个原生 OMML，所要求核心公式结构覆盖通过；限定 WPS 内部字符编辑往返另通过。但架构连线、非核心原始 TeX 记号、公式宽度及第 6 页遮挡仍有问题，独立结论 **QUALITY_FAIL**。                               | [保守上界 US$0.248488](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/budget-audit.json)；SDK 估算约 US$0.15459431，账单未知。             |
| v5 学术手选 Cartesian | 用户显式选择并实际冻结 Cartesian；6 页、107281 字节、SHA `1a11d3ac…f62c8`，13 个完整回执、0 未知请求，正常 completed。两次预览的自动问题数 7→0，显式交付。                                     | completed 与零自动问题已核，但独立审计及 WPS 逐页观察均为 **QUALITY_FAIL**：第 3 页角色正文跨列、第 6 页因果掩码方向表述错误；另有原始数学副标题等问题。4 个 OMML 已覆盖 6 个核心表达式，不能用对象数代替质量结论。 | [保守上界 US$0.156354](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/budget-audit.json)；SDK 估算约 US$0.09676194，账单未知。 |

运行证据分别见 [Auto 终态](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/run-audit.json)、[Auto 独立质量审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-auto/quality-independent-audit.json)及[手选终态](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/run-audit.json)、[手选独立质量审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v5-academic-manual-cartesian/quality-independent-audit.json)。两份真实文件均在右侧工作区逐页访问第 1–6 页，截图分别为 `ui/v5-native-academic-page1..6.png`、`ui/v5-native-manual-cartesian-page1..6.png`；Auto 另有[预览收据](../../../.real-eval/ppt-completion-2026-11-10/native-v5-academic-preview-receipt.json)。翻页没有触发额外生成；可达性不代表布局、语义或全部公式编辑通过。

WPS 12.1.25895 在 Auto 稿的独立私有副本第 3 页，将首个原生公式内部的 `X` 改为 `Z`，保存、关闭、重开后仍在；重开副本仍为 6 页、7 个原生公式，原稿 SHA `bb4e06d1…ecfd4` 未变。首次普通文字 `+0` 尝试已撤销，不计作内部公式编辑。见[实际成品限定往返收据](../../../.real-eval/ppt-completion-2026-11-10/office-edit/academic-v5-auto-wps-receipt.json)。`Z` 是测试性语义改动，该副本不是修正后的科学交付稿；没有逐个编辑全部 7 式，也不认证图表数据、PowerPoint 或 Windows。该正例不能抵消 Auto 稿的独立质量失败。

手选稿另在 WPS 实际只读访问全部 6 页，确认第 3 页角色卡正文跨越分隔线、第 6 页在未定义坐标时写“上三角可见”，与常规 `QKᵀ` 行为 query、列为 key 的因果掩码约定冲突。其 4 个原生 OMML 足以覆盖六个要求的核心表达式，第 3 页将 Q/K/V 合为一个对象；第 5 页四条 head→Concat 几何连线实际存在，问题是共线与框重叠的可读性，不能误记为缺失 head 连线或串行依赖。见[手选 WPS 记录](../../../.real-eval/ppt-completion-2026-11-10/office-edit/academic-v5-manual-wps-receipt.json)。这是显示与内容检查，没有编辑该稿公式；字体警告仍在。

结算这两例保守上界后，此时共享 US$3 账本从 US$1.992699 增至 **US$2.397541**，未预留余额为 **US$0.602459**；生图仍为 **0/最多 1 张**，两条历史未知结果预留继续保留。金额由此前快照加 US$0.248488 与 US$0.156354 得出，是该记录时点的报价上界与预留，不是实际账单。SDK 估算另列、invoice 未核，不把它用于释放未知费用或重置后续额度。本段没有新收费请求或完成标记。

## v6 产品源稿修订、真实单图与安全暂停（2026-11-11）

本例在私有 v6 中引用原产品稿 8 个实际页，以既有内容为基础修订，并明确授权最多 1 张 Pro 概念插画、图片 US$0.05、文字暂停阈值 US$0.10。文字模型仍为 GPT-5 Mini，关闭网页搜索。原稿 SHA `53542a63…2051219` 未变；修订输出各有独立身份，没有把原稿追改成新版本。运行以 `assistant_cost_limit` 结束，不能把预览自动保留误称最终模型复核完成。见[实际运行审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/run-audit.json)。

| 结果            | 实际证据                                                                                                                                                                          | 结论边界                                                                                                                                                                                                          |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 真实单图        | 1 个逻辑请求提交，网关 HTTP 401（已知未计费拒绝）→200，得到 185044 字节 JPEG，SHA `4b47e82938d4d7544071931db821354f762d53b68edbf001c2d82837156bd16d`，已在本地作为独立附件导入。  | 生图与本地导入通过，不等于已嵌入 PPT、图像质量全部通过或实际账单已核。模型的 5 次工具尝试不等于 5 次 provider 生成。                                                                                              |
| 中间修订 v2     | 8 页、39758 字节、SHA `6666a885…efa80`，19 条警告；无 media part、无嵌图。                                                                                                        | 是真实中间产物，不能用单独生成的 JPEG 冒充其封面图。                                                                                                                                                              |
| 最终保留修订 v3 | 8 页、39732 字节、SHA `3861ddcc456f4e37ff7dfa2d73e608a6bb773390e14944f8d4e85b66ca94b10d`，17 条警告，0 个 media part/0 条 image relationship。页序 ID 256–263 与全部 notes 保留。 | 独立结论 **QUALITY_FAIL**：付费 JPEG 未嵌入，继承的标题仍在不透明页头之后且白色格式丢失；多页新旧正文重叠，第 4 页原始字典虽消失，旧大卡片与连线仍叠在新增对象下。保留页序与 notes 是局部正例，不是内容质量通过。 |

见[图片导入审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/image-import-audit.json)与[独立成品质量审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/quality-independent-audit.json)。实际成功代码把聊天附件 ID 当作 `image()` 所需的计划素材 ID，随后用宽泛 `except` 吞掉异常；Host 成功回执已有 `presentationAssetId`，受控绑定成功。通用模型可见指导仍引导使用附件 ID，两个命名空间的衔接存在已确认的指导缺口；本案不证明 Host 素材解析器损坏。最终无嵌图应如实记录，不能只看生图卡完成态。

实际右侧文件工作区到达 v3 全部 8 页，截图 `ui/v6-native-product-v3-page1..8.png`。另从原稿 `53542…` 切到修订稿 `3861…`，没有新增模型或生图调用；这是版本选择与预览可达性，不是新稿科学/视觉或 WPS 编辑验收。标题层级与正文问题由实际 XML、代码和缓存图核对，新修订稿没有据此取得 WPS/PowerPoint/Windows 通过结论。

本例有 13 个完整文字回执、0 个新文字未知结果，文字保守上界 US$0.246775，加 1 张实际图片的报价上界 US$0.050000，合计 **US$0.296775**；SDK 文字估算约 US$0.10194537，图片记录估算 US$0.05，invoice 均未核。见[费用审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v6-product-targeted-one-image/budget-audit.json)。结算后此时共享 US$3 账本保守占用 **US$2.694316**，未预留余额 **US$0.305684**，实际生图 **1/最多 1 张**，单图额度已用尽；此前 US$2.397541/US$0.602459 等检查点及两条历史未知预留仍保留。金额不是实际账单，剩余美元也不重置图片数量授权。

另在源码发现独立的鉴权 **P1**：RunControl 为脱敏保存了 OFox key 与 MCP env/header 私有值，但 5 个媒体鉴权读取点使用 `secrets().last()` 选 Bearer；9 个 OFox key 更新点也需要与纯脱敏追加分离。受影响范围含聊天内图片、视频、语音及独立图片/视频入口。正确边界应是专用内存 OFox key 槽，MCP 只进入脱敏集合，轮换返回后复验账户、代际和取消状态再更新/发送；不能用集合的首项代替末项。此时仅准备合成回归与修复方案，**此 P1 的 RED/修复/通过尚未完成**，既有包与通用测试不能代替该项安全验证。

该静态代码链条与实际 401→200 不能直接等同：没有读取或比较真实 MCP 凭据值，没有证据证明本次已泄露凭据，也不能把 401 原因确定为此 P1。根代理确认私有 v6 已 idle 并正常退出（PID 90014），此后暂停新增收费调用，整改与验证继续；安装版与无关旧任务没有变更。本记录未将本地 todolist 或 PPT 主题标记完成。

## v7 限定安全回归与私有包门禁（2026-11-11）

媒体鉴权已改为专用内存 OFox key 槽，MCP 私有值只进入脱敏集合；异步轮换在接受新 key 前后核对身份、Host 代际和取消状态。私有 Host RPC 的顶层脱敏 sideband 只供匹配的待决调用更新同一运行内存集合，不进入模型工具结果，也不更换模型 Authorization。对更长轮换 key 的分片 SSE，输出前动态重算保留尾长。独立源码复核检查了这些限定边界；[1956 项源码核对记录](../../../.real-eval/ppt-completion-2026-11-10/v7-source-security-independent-review.json)只是当时的冻结与源码结论，不能代替后续 wire 门禁。

保留首次门禁失败：[私有 frame 回归](../../../.real-eval/ppt-completion-2026-11-10/v7-private-host-frame-green.log)为 1 通过，但同轮[实际 Host wire 回归](../../../.real-eval/ppt-completion-2026-11-10/v7-auth-final-wire-green.log)为 2 通过、1 失败。合成 OFox A→B→B 鉴权及公有事件脱敏通过后，原生 JSONL 仍包含合成 MCP canary。源码定位是首次运行遗漏 `knownSecrets`，只有整轮鉴权重试才从 RunControl 播种；MCP 自身协议 cleaner 不覆盖模型普通回文。[初始集合补丁](../../../.real-eval/ppt-completion-2026-11-10/ofox-key-slot-security-draft/08-seed-trusted-run-scrub-list-after-red.patch)与[无轮换回归](../../../.real-eval/ppt-completion-2026-11-10/ofox-key-slot-security-draft/09-initial-mcp-native-echo-regression.patch)经独立静态复核后已落地。后续[实际 Host wire 3/3](../../../.real-eval/ppt-completion-2026-11-10/v7b-auth-wire-green.log)通过：两种 apex 的图片、视频/语音鉴权，首次无 401/MCP 调用的原生长 canary 脱敏，以及轮换 A→B→B 的公有/事件/原生记录均有实际合成回归；16.30 秒、退出 0。首次失败不追改为首次通过。

新 [v7b 源码冻结](../../../.real-eval/ppt-completion-2026-11-10/v7b-final-source-freeze.json)含 1956 项，SHA `dbb6447d5dfccb7f260a4c6e19a42f64f93c91c1f9c5c007e326a0c99e8c3a32`。Rust 最终全量 **4514 通过、0 失败、27 ignored、22 个结果组**，其中 library 4276/25 ignored、Host 122/2 ignored；fmt 与 all-targets Clippy 退出 0，见[汇总](../../../.real-eval/ppt-completion-2026-11-10/v7b-final-check-summary.json)。运行时完整 **710/710 通过、0 失败/跳过/取消**的[收据](../../../.real-eval/ppt-completion-2026-11-10/runtime-v7-guidance-final-receipt.json)覆盖 326 项源码/技能/测试；随后只改变 Rust 初始化和 Host 测试，该 326 项仍逐项相同，故复用此门禁，不称新跑一次。前端同样复用 [3323 通过、1 跳过、0 失败及 typecheck/format](../../../.real-eval/ppt-completion-2026-11-10/frontend-post-live-fix-receipt.json)，1083 项前端/测试/配置与当时记录一致；不是重新执行前端全套。

正常签名构建、私有复制和 deep/strict 核对共[六阶段](../../../.real-eval/ppt-completion-2026-11-10/v7b-v7-private-check-build-pipeline.json)退出均为 0，流水线 636.175 秒的 release 阶段包含 prepare/renderer 构建。私有 v7 主程序 SHA `ae8f8b1434e58d1aff9e6ca3b72c25cdd94ab9735126d9b48a8ad3239caa20e8`；[静态审计](../../../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v7-1794359945682.json)无发现，收据 SHA `356395a9f6f2e745af2f8ba9ceb3e4053a468d8c89ec4c1ef75b62a00529f81f`。9 个 Mach-O（含 Node/Canvas）的 Team、DR、hardened runtime 与 entitlement 检查通过；源码→staged→bundle 逐字节相符，运行时 11854/11855 个文件的唯一额外声明项为 `PI_LICENSE`；renderer 234 个 asset keys、233 个原始 payload 加转换后的入口 HTML 对应全部 234 个 dist 文件。安装版仍为 `810f6031…1d57a93`，没有替换、发布或新公证；流水线的 `appLaunched:false` 是复制时历史状态，不作为后续 GUI 收据。本段只记录构建、来源与限定安全门禁，不包含新 v7 模型成品或原生质量通过，不追加新案例费用。

另有明确接受的覆盖限制：[通用内容脱敏](../../../assistant-runtime/src/security.mjs)当前只替换长度至少 8 的已知字符串，而 [MCP 凭据识别](../../../assistant-runtime/src/mcp-security.mjs)可以识别更短的密码。当前长 canary/轮换回归不证明短 MCP 凭据在原生记录和模型上下文中均完整脱敏。本轮没有扩大通用协议控制标量的改写范围，不声称“所有凭据均已脱敏”或整体安全无缺口。全部安全回归使用合成值；没有读取实际 MCP 凭据，也没有建立实际凭据泄露事实。

## 零 API 离线作者控制稿与 WPS 只读正例（2026-11-11）

为区分运行链能力与模型成品质量，另编写明确标注 **OFFLINE AUTHORED ADAPTATION** 的确定性控制稿：[结构与视觉记录](../../../.real-eval/ppt-completion-2026-11-10/offline-focus-v7-control/structural-visual-receipt.json)。输出 8 页、193447 字节，SHA `9f8692abaea9b7a252d177e05f774a6fe76e825c8ad5b395888870351966cb20`；原 8 个 slide IDs 与 notes 保留，2/3/5–8 页除标题顺序外的 XML 不变。原生标题 ID 2 及全部 run 格式保留，7 个内容页标题移到不透明页头之后。封面只复用已有 JPEG `4b47e829…bd16d`，没有再次生图；第 4 页先清除旧正文，再建立 3 个紧凑原生节点和 2 条带方向的原生连接线。独立复核比对了 ZIP 结构、全部 notes/title runs、总览及第 1/4 页全图。

根代理在 WPS 12.1.25895 实际只读查看全部 8 页，确认标题可见、封面嵌图和第 4 页两条向右箭头；前后文件 SHA 不变。见[WPS 收据](../../../.real-eval/ppt-completion-2026-11-10/offline-focus-v7-control/wps-readonly-receipt.json)，SHA `435e4a88132ea3f99d99eb360869178fcf8316d48597d64971268c5d1186f25a`。封面“工作”跨行和 WPS 缺字体警告仍保留；未执行该稿编辑/保存/重开，未验证 PowerPoint/Windows。此控制稿新增 API 调用为 0，不改共享账本，**不算 Desktop/model 质量通过，不修写真实失败产物，也不把 PPT 主题标记完成**。

## v7 两次 Nano 原生流程失败与当前预算（2026-11-11）

私有 v7 已通过正常 UI 启动，实际进程路径及主程序 SHA 与构建副本一致，见[启动收据](../../../.real-eval/ppt-completion-2026-11-10/native-v7-launch-receipt.json)。随后使用 `openai/gpt-5-nano`、Product Launch 手选、原稿 8 页引用和既有唯一图片做两次普通发送，生图 off/0。两次实际原稿全页浏览与引用可达，但原稿可预览不能代替新稿交付；源 SHA `53542a63…2051219`、既有 JPEG `4b47e829…bd16d` 均未变。

| 实际运行                                                        | 实际终态与证据                                                                                                                                                                                                                                                              | 结论边界                                                                                                                                                                                                                                                                                                  |
| --------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| 首次 Nano 源稿修订，run `1a1c668f…`                             | 文字暂停阈值 US$0.03；7 个完整 HTTP 200 回执。2 次 freeze、2 次 create、3 次 candidates 尝试中有 6 次执行前 schema 拒绝和 1 次当前计划拒绝，终态 `assistant_cost_limit`，operation failed。错误含 `diaphrams`、缺 style/name/format/scenario、source 错层级及使用历史计划。 | 没有成功冻结计划、新生成或新产物；未到达 Python/PPTX 引擎，不是实际引擎执行报错。见[运行审计](../../../.real-eval/ppt-completion-2026-11-10/native-nano-v7-product-source-reuse/run-audit.json)。                                                                                                         |
| 人工提供完整计划与离线校验代码的 Nano 辅助流程，run `9b62f8ce…` | 实际发送阈值 US$0.04；6 个完整 HTTP 200 回执。模型连续 5 次提交与提供的 `01` 完全一致的 candidates 参数：前 4 次成功返回 eligible、`product-launch` 和明确的 freeze 下一步；第 5 次命中正常重复调用门禁。末尾同 run 助手消息为 `assistant_repeat_limit`，operation failed。 | 没有调用 freeze/create，没有提交或执行所提供代码、没有 previewId 或新产物。成功候选反馈在本地原生工具文本中存在；不能归因于提供 JSON 的 schema 错误或 PPT 引擎失败，也不猜模型隐含动机。见[运行审计](../../../.real-eval/ppt-completion-2026-11-10/native-nano-v7-assisted-source-reuse/run-audit.json)。 |

第二次实际消息体 SHA `17796a567ec3d5de40907a2a144d355aa25ae6b80d465dd2f830cdd6f10a239c`；早先未发送的 US$0.02 草稿在发送前已替换，不能把它当实际策略。US$0.239862 是发送前任务预留，不是实际费用。私有审计最初误取同 run 首个 completed 助手段，现已改为最后一条 error 并列出两条记录；这是审计选择器更正，不是产品完成态错误。两例说明当前已测模型的工具编排可靠性不足：第一例参数/历史计划错误，第二例即使有明确有效反馈和完整人工计划仍反复停在第一步。第二例本来就是人工辅助功能测试，不能称自主设计通过；两例也都没有新成品可作视觉质量判定。

首次文字报价保守上界 **US$0.043250**、SDK 估算约 US$0.03253561；第二次上界 **US$0.023796**、SDK 估算约 US$0.01726515。两次新增未知结果均为 0，图片/视频/语音/搜索/MCP 实际工具调用均为 0；没有新增生图。见两份[首次费用审计](../../../.real-eval/ppt-completion-2026-11-10/native-nano-v7-product-source-reuse/budget-audit.json)、[辅助流程费用审计](../../../.real-eval/ppt-completion-2026-11-10/native-nano-v7-assisted-source-reuse/budget-audit.json)。两例结算时共享 US$3 的保守占用为 **US$2.761362**，未预留余额 **US$0.238638**，无活动预留，实际生成图片仍为 **1/最多 1 张**；历史未知预留 **US$1.057077** 继续保留，实际账单均未核。彼时[身份/空闲/预算检查](../../../.real-eval/ppt-completion-2026-11-10/v7-idle-final-identity-budget-check.json)另确认 1956 项源码未变、私有包与安装版身份不变、activeRun 为空及新预算未准入。此前全部费用检查点保留，不把失败工作流的实际模型调用记为 0。

后续离线控制稿导入尝试在任何导入动作发生前被锁屏阻断，没有补出新的导入或编辑正例，既有 WPS 八页只读控制稿证据不变。当时的[可审阅提案](../../../.real-eval/ppt-completion-2026-11-10/next-native-gpt5mini-reviewable-proposal.json)请求把总预算从 US$3 增至 US$4，仅用 GPT-5 Mini 复验一次既有源稿/图片的人工辅助流程，不新增生图、不安装或替换应用。提案采用 US$0.08 文字暂停阈值、10 倍 SDK→保守报价系数，加完整终端最多 3 次 POST 与记账余量，拟预留 **US$1.189608**。该提案文件保留形成时“未准入”的历史状态；用户随后明确同意总额 US$4，根代理重新核价、预留并执行的实际结果见下节，不再把此历史检查点误写成当前仍待预算授权。

## GPT-5 Mini 真实人工辅助功能正例与限定 WPS 往返（2026-11-11）

用户明确批准文字与图片合计总预算从 US$3 增至 **US$4**，最多 1 张生成图片的数量限制不变，已生成的唯一图片只复用。冷重启前后免费 V1 目录逐字节相同，SHA `c123c8f3…6933400`，见[前](../../../.real-eval/ppt-completion-2026-11-10/v7-gptmini-cold-catalog-before.receipt.json)/[后](../../../.real-eval/ppt-completion-2026-11-10/v7-gptmini-cold-catalog-after.receipt.json)收据；这与本轮实际请求的价格快照一起确认输入/输出/缓存读每百万 token 为 US$0.25/US$2/US$0.03。私有 v7 进程 PID 14344 的路径和主 SHA `ae8f8b14…caa20e8` 未变，安装版未替换，1956 项源码保持冻结。按 US$0.08 暂停阈值、图片 off/0、关闭搜索，以原稿全部 8 页及既有 JPEG 做一次新普通发送，见[实际发送快照](../../../.real-eval/ppt-completion-2026-11-10/v7-gpt5mini-assisted-source-reuse-sent.json)，不是重试旧 Nano 操作。

实际 run `265d5ddc…` 正常 completed，operation completed。模型调用 freeze 1 次、create 2 次，取得本轮计划 `4b769dd8-9ed0-4796-b314-c3972bc2a79c`、revision 1、Product Launch，并以本轮刚返回的 previewId 交付一次实际新文件，未再次生成。见[运行审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/run-audit.json)。本例没有显式调用 `get_presentation_candidates`；freeze 入口自身的 `checkedPlan` 仍重新执行候选及手选硬约束，不能把内部验证写成模型显式走完候选步骤。实际计划末页构图文字有简繁字差异，实际代码 SHA `66a48fd0…f81190b` 与提供的 `6dcdbbae…812f423` 不同：模型添加了 `getattr` 和宽泛异常捕获。不能声称参数/代码原样转发，也不把这些异常吞掉路径评为通用可靠修复。

实际 Native 输出 `Focus_V7_人工辅助功能复验.pptx` 为 **8 页、193447 字节**，SHA `757cf3afc05eb84dabf82c25ea0d4b58f2f32173063ef44bca59d3a5c72da728`。原 slide IDs 256–263、8 份 notes、8 个标题 ID 2 与全部 `a:r` 富文本保持，7 个内容页标题高于页头；2/3/5–8 页的非标题对象与原稿不变。唯一 media 为原 185044 字节 JPEG `4b47e829…bd16d`，只在封面出现；第 4 页旧正文已清，改为 3 个紧凑原生节点与 2 条原生箭头。全部解包 part 与已验证 UUID 控制稿相同；这是独立核对的实际产物，不是拿控制稿替换输出。[独立结构与八页重新渲染目检](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/quality-independent-audit.json)收据 SHA `c91666fe74df55d7a72d0b791da4d4de79978732633b60942c15d7ea623c3d2f`，限定范围没有新增阻断；兄弟审查另比对 ZIP 并实际查看八张 sidecar 图。封面“工作”跨行、原稀疏版面/较小正文和字体替代仍是限制，不能升级为出版级设计或自主 Auto 质量通过。

本轮原生工具结果实际包含 2 个 JPEG 图像块，固定 SDK 转换器可把这些精确像素转换为图像输入，见[视觉输入证据](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/vision-input-audit.json)。没有截获远端请求 body、证明 provider 解码或证明模型逐页完成视觉复核，不能把“有图像输入”写成“模型视觉质量已验收”。根代理另在 Desktop 实际查看了该新稿全部 8 页，截图 `ui/v7-gptmini-native-page1..8.png`，见[Desktop 检查](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/root-desktop-check-receipt.json)。

最初[WPS 局部收据](../../../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-partial-receipt.json)只记录了实际新稿第 1 页和独立图片选择，随后锁屏阻断剩余查看；该历史收据保留，较早 Desktop 收据的 `wpsNotYetOpened:true` 也不追改。此后根代理在 macOS 26.5.1 (25F80)、WPS 12.1.25895 **实际逐页查看 1–8 页**，确认 8 个标题可见、封面图片仍可单独选择，第 4 页 3 个原生节点和 2 个向右箭头显示正确。见[本轮实际 WPS 收据](../../../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-receipt.json)，SHA `aaf8c012ada3df012cd76e4d8f818daa10ead5d36e7e91c72a588b6640912314`，含 12 张实际截图。

仅在独立副本 `office-edit/product-v7-gpt5mini-wps.pptx` 的第 4 页中间流程节点内，把普通原生文字“选择目标”改成“选择本次目标”，Command-S 保存、Command-W 关闭至 WPS 首页，再通过 Finder 精确同一绝对路径重开，替换文字及 3 节点/2 箭头仍可见。截图 `ui/wps-v7-gptmini-node-reopened.png` 与副本 XML 均支持该限定往返。生成原件保持 193447 字节、SHA `757cf3af…c72da728` 不变；WPS 副本从同一 SHA 变为 **230671 字节、SHA `8176ac5ec5b6bf5c5ded158f8bddd8f9b4b7bf13b0bf4d4c1e7fad4a4064490b`**，不把编辑后的副本冒充原始交付。

[独立保存后结构审计](../../../.real-eval/ppt-completion-2026-11-10/office-edit/product-v7-gpt5mini-wps-independent-audit.json)，SHA `8d33c3343470dc36eb60ee404ee50d1e86eba74cc187a38dc903e634b27dad5d`，核对 8 个 slide IDs、对象 IDs/z 序/直接几何、3 个节点属性与 2 条箭头规范 XML 保留；唯一普通文字内容变化是第 4 页 textbox ID 14。JPEG 只由 `.jpg` 更名 `.jpeg`，185044 字节与 SHA 不变。**WPS 保存不是单一 XML 补丁**：52 个同名文件 part 被重写、增加 16 个目录条目，8 份 notes 的文字序列保持但原始 XML 均重写；东亚字体声明从 `Microsoft YaHei` 转为“微软雅黑”，另加 lang/panose/default 等声明。标题逻辑字号/粗细/颜色保持，不等于保存后 `a:r` 或 notes 逐字节不变；上文 Native 原件的字节保持结论不能套到 WPS 副本，也未在重开后逐页再看全部 8 页。

本次通过的是**这份真实 Native 新稿的八页显示与一处原生节点文字编辑/保存/关闭/重开**，不是所有对象编辑验证；未移动节点/连接线，不推导图表、数据单元格或公式通过，该产品稿没有原生图表/公式。WPS 缺字体警告与封面“工作”跨行未解决，PowerPoint/Windows 未验，仍不构成自主设计或整体质量验收通过。旧离线控制稿的 WPS 八页正例继续单列，不用于代证本次真实文件。本轮 WPS 操作新增模型/生图调用与费用均为 0。后续自主学术案例最初只创建了手动命名的空会话并设置 Auto/off0/US$0.06；[未发送检查点](../../../.real-eval/ppt-completion-2026-11-10/v7-academic-autonomous-not-sent-lock-checkpoint.json)保留当时锁屏、0 messages/operations 和无活动运行/预留的状态。之后实际发送与失败另记下文，不能把该早期空会话快照当当前状态。

本例 4 个文字回执完整、无新增未知结果；按实际 `.25/2/.03` 快照、全部输入含缓存按最高输入价保守计，逐笔向上取整的报价上界 **US$0.081768**，SDK 估算 **US$0.05062229**，invoice 为 null。见[费用审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-assisted-source-reuse/budget-audit.json)，根代理已独立复算并结算。该例结算时共享 US$4 保守占用 **US$2.843130**，未预留余额 **US$1.156870**，无活动预留，当时图片限制仍为 **1/最多 1 张**，历史未知预留 US$1.057077 不释放。本例图片/视频/语音/搜索/MCP 新调用均为 0；不把重用付费图当新生图。先前“US$4 未获准”的提案状态已被用户批准及本例执行取代；此后新授权与案例按下文分别记录。该实际人工辅助功能正例不清除历史 Auto/手选失败，不改变来源 41 页/partial 边界、PowerPoint/Windows 未验状态或 PPT 主题未完成状态。

## 新自主学术例的真实引擎失败与 v8 有界修复

用户解锁后，新空会话实际以普通发送执行了六页自然语言纠正性任务，SID `a7ce7ed6…`、run `f1ff9140…`；没有提供页面 JSON 或已验证 Python 代码。它不是原提示不变的因果 A/B。实际 3 次 schema 拒绝涉及 visualNeeds 数量、density 和 relationship 后，模型取得本轮 Course Module 计划 `5c22ba79-bd00-418b-b807-cde3617fe6c7`、revision 1。第一次 create **真正进入 Python**，在 `equation()` 解析 `\sin` 时失败；第二份修订代码的 create 被费用暂停门禁挡在执行前。与此前 Nano 未触及引擎的失败不同，本例有明确 helper 不支持命令的真实执行错误，但没有新文件、预览或成品质量通过。见[运行审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/run-audit.json)。

8 个完整 HTTP 200 文字回执、0 个新未知结果，独立逐笔向上取整为 **US$0.079948**，SDK 估算约 **US$0.07107410**；阈值 US$0.06 是用量后的暂停触发值，不保证 SDK 总额不会越过它。根代理[独立结算](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/root-settlement-receipt.json)后，当时累计保守占用 **US$2.923078**、未预留余额 **US$1.076922**；[费用回执](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v7-academic-autonomous-corrective/budget-audit.json)保留 invoice 未知与历史未知预留，没有把无文件当零调用。

v8 只给有界解析器增加 `sin/cos`→既有直立 roman 结构、`quad`→U+2003 EM SPACE；未知宏、深度/数量限制和失败前不改 slide XML 的边界不变，不新增 TeX 解释器、依赖、文件/网络能力。`quad` 固定的是字符，不保证跨字体/Office 物理间距。实际 RED 为 2 失败/1 通过，之后 36 项相关聚焦测试通过并完成独立源码审查；Skill 正文 25795 字符，LaTeX 才附 PNG，structured/literal 保留旧文本回退。规划指导另经 RED→41 通过，澄清必填 density、文档 visualNeeds 上限和 `sequence` 的字段含义；枚举/门禁没有放松。见[有界修复收据](../../../.real-eval/ppt-completion-2026-11-10/math-sincos-v8-proof/implementation-receipt.json)。

原 4017 字符失败代码在新 helper 下**零 API 原样离线重放**可生成 6 页、5 个 OMML、schema 0 错误，但仍有 4 条公式宽度警告及普通 `\cdot` 字符串的非法转义警告，未作质量接受，也没有给失败原生会话补附件。见[离线重放](../../../.real-eval/ppt-completion-2026-11-10/math-sincos-v8-proof/offline-replay-receipt.json)。另一个一页自编数学 probe 在 WPS 12.1.25895 内部把第三式第一个 x 改成 z，保存/关闭/同路径重开保留；4 个 OMML/4 个回退仍在，第三张 PNG 也确实更新成 z。原件 `1d527bcb…0219fa` 未变，67281 字节副本 SHA `b726551d085e1c92700ca726afe1c25df5849f83acf44c160dc74fb44547b27a`。见[实际 WPS 记录](../../../.real-eval/ppt-completion-2026-11-10/office-edit/sin-cos-v8-wps-receipt.json)与[独立审计](../../../.real-eval/ppt-completion-2026-11-10/office-edit/sin-cos-v8-wps-independent-audit.json)：25 个同名 part、全部 4 个 PNG 被重写，第三式几何改变，不能称 XML/排版逐字节不变。只改一个变量会破坏原恒等式，这份副本仅作编辑 probe，不是正确科学交付、模型讲解验收或任意跨引擎保证。

## v8 冻结态门禁与私有运行时资源更新

[最终汇总](../../../.real-eval/ppt-completion-2026-11-10/v8-final-check-summary.json)对应 [1957 项冻结](../../../.real-eval/ppt-completion-2026-11-10/v8-final-source-freeze.json)，SHA `e7b55c907c8e47247809208a6c1d880fe2ec44888848b1cffa32917d77df48de`：runtime **715 通过、0 失败/跳过**；TS typecheck/format 退出 0。前端最初并发运行保留 2 个 unchanged `LibrarySourcesWorkbench` 失败，该文件聚焦 33 通过后，以 2 workers 重跑全套 **3323 通过、1 跳过、0 失败**，未改生产代码，不把负载疑因当已证实的唯一原因。见[前端复查](../../../.real-eval/ppt-completion-2026-11-10/v8-frontend-recheck-receipt.json)。Rust fmt/all-targets Clippy 退出 0；首次磁盘预检阻止 full 启动的失败保留，清理已完成的 release 中间产物后，resume 全套退出 0，**4514 通过、27 ignored、22 个结果组**。

私有 v8 **没有重新编译 Rust release**：以已验证 v7 私有完整包为基底，仅更新 `document-tools.mjs`、`presentation-planning.mjs`、`python/deck.py` 和 PPT Skill 四份运行时资源后正常 Developer ID 签名。见[资源更新记录](../../../.real-eval/ppt-completion-2026-11-10/v8-private-runtime-update-receipt.json)。主 SHA `c420b4578cbc2e4e79e7a3cfe123ce051e79a8a1c19b3e5553bef40b2c6434a2`，[静态审计](../../../.real-eval/ppt-completion-2026-11-10/private-full-desktop-static-audit-v8.json)SHA `143a150d40b7404abee2529329a622a24a6d0f32b07c93236c3edde78a0422dc` 无发现，9 个 Mach-O/深严签名、资源/renderer 核对通过，v7/v8 主程序签名前完整字节前缀相同。安装版未替换、未发布/新公证；v8 后续已实际启动，历史资源复制收据的 `launched:false` 和审计内先前门禁快照不追改。

这些是**该 v8 冻结态**的历史门禁。随后定位实际原始第 4 页 SVG 含三组 marker/line、但 Skia 兼容 JPEG 丢失箭头头，以及 Architectural Plan 封面 metadata 与自身面板页码相撞的 helper 缺口。两项在当时尚待修复；现已按本文开头的 v8b 独立回归、最终完整门禁及私有候选包记录闭合，不归因于模型自画，也不把旧 715/3323/4514 或旧签名直接当新 gate。

## 多图授权、报价准入与 v8 自主产品终态

用户随后明确取消全局“最多 1 张生图”，允许按每页用途自适应多图，文字可考虑 DeepSeek Flash；原 **US$4 总额不变**。账本 `maximumGeneratedImages=null` 表示旧全局数量限制已撤销，不取消每轮冻结数量/USD、Host 逐素材检查、历史未知预留或总余额限制。图表/公式/流程仍应采用可编辑原生对象，图片上限不是配额，0 张也可合理。见[免费报价/准入评估](../../../.real-eval/ppt-completion-2026-11-10/adaptive-multiimage-deepseek-admission/adaptive-admission-review.json)。

当前 DeepSeek V4.1 Flash 的已核目录是 1M context/384k output、cache read US$0.006/M；实际普通请求没有更低的固定输出 cap，不能假设 16k/32k。最高输入/输出 US$0.30/US$1.20/M、cache 折扣因子 50，最低 UI 阈值 US$0.01 加最多 3 次完整 POST 的保守预留为 **US$2.783400**，超过彼时 US$1.076922 余额，因此未准入/发送。不能以“提示很短”替代费用上界。Mini 的 0–3 图/US$0.15、文字 US$0.05 候选预留 US$1.039608 在重新核价和冻结后单独执行，不同时准入多份最坏预留。

实际 v8 普通自然语言新稿 run `675aafb1…`，无源文件引用、无人工页面 JSON/代码，Auto 选定并冻结 Architectural Plan `f2bd2589-2d2f-4294-83bc-cf1c2b5b2013` revision 1；这是部分原生风格适配，不是完整上游模板转换。[发送快照](../../../.real-eval/ppt-completion-2026-11-10/v8-product-adaptive-sent.json)为 auto/max3/US$0.15、text US$0.05、默认精确 Pro 模型。5 次生图工具尝试中前 3 次是本地参数拒绝、未形成付费提交；后 2 次 gateway 200 真实生成两张图，分别在第 1/2 页逐字节嵌入。8 页 PPTX **650768 字节、SHA `1d9a0da6b52d0fd33e3583567adb5d8ab462996415c5b297cda1d4dd13210357`** 在首次成功预览后自动保留交付，第二次布局修订被 `assistant_cost_limit` 挡住，operation failed；不能把存在文件写成正常 completed 或完成模型终审。见[实际运行](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/run-audit.json)。

[完整八页质量审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/quality-audit.json)，SHA `ada673312d5af772ab5d9f320777d1dd392554d5a8b9642b1e96fad9188ec2fd`，结论 **QUALITY FAIL**，保留“真实多图生成→指定页精确嵌入”的功能正例。第 1 页后置图片在 WPS 遮挡标题末字，页码 01 与团队页脚冲突；JPEG 文字置顶会掩盖前者。第 3 页把“不同步外部应用”“静音模式”等扩展未单独标拟议；未发现虚构客户/价格/发布/量化收益。5 条卡片高度警告是框过高、内容过空，不是文本溢出；第 6 页计划工作板/时间线未兑现，6–8 页重复稀疏。不要为填满版面编造内容。原生第 4 页连线方向和三角端点在文件中存在，但未绑定到节点；JPEG 箭头显示与真实 Office 分开判断。

根代理在 Desktop 实际查看原始 8 页，最后 next 禁用，见[UI 收据](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/desktop-ui-receipt.json)。另在 WPS 12.1.25895/macOS 26.5.1 逐页看原稿 1–8，再对独立副本手动下移第 1 页独立图片以解除遮挡，并把第 4 页“整理任务”改成“整理今日任务”，保存、关闭、Finder 同路径重开，两项修改保留。见[WPS 记录](../../../.real-eval/ppt-completion-2026-11-10/office-edit/product-v8-adaptive-wps-receipt.json)，SHA `0c964ee2f40044323f0d849fd4bf763463972a3e480910c42ebf5f30dd7cadde`。副本 698558 字节、SHA `1020a5f9883436fe6176002322cd833e8eb0c2fddaf2e84097efedc2f0498336`；生成原稿保持 `1d9a…10357` 不变。人工修正和限定图片/文字编辑往返不使自主原稿质量通过，也不证明所有对象、连接绑定、图表/公式或 PowerPoint/Windows 通过；缺字体提示仍在。

[独立 WPS 审计](../../../.real-eval/ppt-completion-2026-11-10/office-edit/product-v8-adaptive-wps-independent-audit.json)，SHA `702bcdd6c944e31251d92d4455aafd24e0e7cd43209c9949fb61280ed66d2d43`，核对唯一普通文字变化为第 4 页 textbox ID 12，唯一直接几何变化为第 1 页图片向下 561340 EMU（约 0.614 英寸），图片尺寸和两张 JPEG 字节未变；8 个页序 IDs、对象 IDs/z 序、8 份 notes 文字及原生连线属性保留。WPS 重写 notes/title XML、字体声明、关系和媒体文件名，不能称原包/富文本逐字节不变，也不能据重开两页推导保存后全部页逐页目检通过。

12 个完整文字回执、0 个新未知结果，文字报价保守上界 **US$0.113663**，SDK 估算 **US$0.05550785**；加两张 Pro 报价 **US$0.100000**，本轮合计 **US$0.213663**，不是实际 invoice。见[费用审计](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/budget-audit.json)与[根代理结算](../../../.real-eval/ppt-completion-2026-11-10/native-gpt5mini-v8-product-adaptive/root-settlement-receipt.json)。原 US$1.039608 只是活动预留，现释放未用 US$0.825945，当前保守占用含旧未知为 **US$3.136741**、未预留余额 **US$0.863259**，无活动预留，累计真实生图 **3 张**、全局 max=null；旧未知 US$1.057077 未释放。所有旧失败、源模板 41 页/partial 边界和未完成 TODO 保留，未安装/替换应用，后续运行须重新准入。

## 独立驱动准备记录（已停止使用）

初始未授权预算时仅做离线验证。2026-11-11 用户明确授权文字与生图合计最多 US$3、最多1张生成图片，已记录`live-budget-authorization.json`。四例计划为学术同内容自动/手动各6页、经营8页、产品8页；计划不等于已生成案例。已核对所选地区公开模型目录及计价依据，准备共享持久化预算账本、逐请求身份复核和显式启动的真实 Host 驱动。文字采用 DeepSeek V4.1 Flash；产品例可按需使用固定报价 Qwen Image 3.0，最多1张。GPT Image 2 未取得可用的单张费用上界，本轮未选择它；其余三例关闭生图。未知使用量保留整次预留，不重置账本或自动重跑失败案例。

切换到原生UI之前，独立驱动的付费请求为0。安装版的 release 构建收据、主程序固定字符串及只读条目查询均指向正式钥匙串，未发现误用 debug FileStore。用户已确认登录钥匙串解锁后，进一步定位：根代理先前的 MCP Node REPL 缺少标准 `process`，读取尝试在访问 OS 钥匙串前就退出；此前让用户检查解锁状态的诊断不够准确。改为现有本机 Node 22 独立进程，先通过不读取凭据、不联网的 `--check`，再经独立审查以显式开关执行。此次实际 OS 读取等待30秒后超时；界面工具明确禁止访问 `com.apple.SecurityAgent`，不能代用户处理安全窗口。该历史尝试不代表整个任务的当前费用为0。

没有修改钥匙串权限、复制正式凭据到开发存储、替换安装版、创建新密钥或由独立进程获取到可用密钥。独立驱动的真实需求→模型选型→逐页计划→生图决策→成品复核未执行，不能写成自动路由、生图质量或收费模型费用通过。独立入口 `live-run-root.mjs` 的凭据只允许保留在同一进程内存，最终释放；固定持久账本及已有消耗不能被新输出目录重置，失败案例不自动重跑。

用户随后确认看到系统提示、可配合下一次授权。再次30秒 OS 读取仍超时，故仅在私有入口增加一次120秒人工确认等待适配器，经独立审查确认固定命令、正式槽位、8KiB缓冲、脱敏与清零、前后身份复核均未改变，也没有 UI 自动化、ACL 修改或重试循环。120秒读取仍以 `timeout_or_terminated` 结束，不能推断密钥不存在、口令错误或 OS 明确拒绝。根代理停止自动读取。切换到Desktop原生生成后，该独立入口已在任何凭据读取前退役，先前的终端启动建议不再适用。独立入口从未建立收费账本或锁文件、付费始终为0；当前Desktop真实调用统一计入上文共享预算。工具拒绝访问安全窗口与 OS 读取超时分别记录，不将二者误写成已证实的因果关系。

私有预算转发、凭据元数据隔离和四案例控制器共41项离线回归通过。真实付费驱动另补4个先失败后通过的收据回归，并在 Cargo 中4/4通过（`live-driver-receipt-cargo.log`）：实际页数来自文件的规范 OPC manifest，不用生成记录声明的页数兜底；手选禁止 Auto，并核对最终生效风格；检查尝试和有效检查分别记录，均不代表视觉通过。驱动编译通过不等于已调用真实模型。新增收据修正后 `cargo fmt --check` 与 `cargo clippy --all-targets -- -D warnings` 再次通过（`clippy-live-receipt-final.log`，73s）。测试时只给 Host 随机本地代理 token，真实凭据只允许保留在内存闭包；共享账本及每次请求的 owner/apex/keyId 复核不能以测试注入绕开。

办公软件和原生WebView后续结果已按上文补录；WPS数据单元格编辑仍未确认，PowerPoint受订阅限制，Windows未验。无人工协议插话的新源码自动选型到合格成品尚未通过，生图质量/费用、任意复杂外来原稿、第三方字体一致性、HTML外部依赖打包和所有上游布局等价转换仍未验证或尚未支持。`live-acceptance-plan.json`是已退役独立驱动的历史计划，不能用其中旧的零调用或凭据前置状态覆盖当前Desktop共享预算。通过真实验收之前保留主题未完成状态。
