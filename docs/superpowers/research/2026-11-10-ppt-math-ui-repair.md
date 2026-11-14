# PPT 公式、生成反馈与交互页面修复

本轮处理实际生成文件中暴露的问题，保留现有 Python / OfficeCLI 引擎、风格目录与许可边界。未把候选资源数当作可用模板数，也未实施真实模型 A/B/C/D 实验。固定模型质量收益、费用与 Windows 原生兼容仍待验证。

## 已核实的原因

- 用户实际答辩文件第 8 页调用旧 `formula()`，三条公式只是普通等宽文本，没有 OMML、分式或上下标结构。同一最小复现能同时得到空 lint、零 schema errors 和零 OfficeCLI issues，说明原“0 个格式问题”不能证明公式正确。
- PPT 技能虽介绍 `equation()`，完整工作示例仍调用 `formula()`，给模型相互冲突的示范。OfficeCLI 对原生 OMML 的预览也会将分式展平成连续文字。
- 生成过程存在空白 reasoning 容器、英文模型状态；成品卡缺少用户可翻页的预览。已保存的历史模型文字不会自动重写。
- 实际 HTML 景点页面已有分类筛选，但缺少搜索组合、结果计数、空结果反馈、重置与可访问的选中状态。它不是整个交互页面生成质量的代表样本。

本地原件、原始生成代码和失败最小复现保留在忽略目录 `.real-eval/ppt-wps-repair-2026-11-10/`，未写入版本库，也未修改原附件、聊天数据库或原打开文件。

## 实现与设计决定

1. `formula()` 保持旧文本兼容，始终给出未排版公式警告。`equation()` 增加有界 LaTeX 子集，未知宏拒绝处理，求和主体要求显式分组；支持分式、上下标、帽号、Greek 和求和。输出原生 OMML，并使用现有 Matplotlib Mathtext 生成 PNG 图片填充形状作为兼容回退；没有引入 TeX、浏览器运行时或新依赖。PNG 与文本回退均不能当作可编辑公式。
2. 更新 PPT 工作示例，使用真正的 `equation()`；独立规划页面、风格选择与检查规则仍保留。自动结构/布局检查、实际视觉检查、目标应用编辑验收分别表达，零自动问题不再泛化为全部通过。
3. OfficeCLI 在临时预览副本中使用已有数学回退，交付 PPTX 保留原生结构。副本失败须产生检查未完成提示。实物 JPEG 检查又发现，本例 OfficeCLI PNG 填充 pattern 经当前 Canvas 缩略图路径后公式不显示，虽然 SVG 已含图片节点；预览 worker 因而只对严格匹配、同宽高比的普通矩形做有界直接图片适配，前后保留 SVG 安全检查，拒绝额外继承绘制语义。没有改写交付 PPTX，也没有据节点数宣称公式已显示。
4. 新 PPT 成品可缓存受限 JPEG 页预览，显示首图并在对话框翻页；只通过现有账户、会话与附件权限开放 JPEG，不执行模型 HTML/SVG。旧记录或预览失败仍可保存文件。关闭、切换权限或会话时清理预览状态，保留键盘焦点恢复。
5. reasoning 空白内容不再占据卡片；真实工具阶段提供 zh/en/ja 文案，不制造百分比。新生成指导要求沿用会话语言，历史模型正文保持原样。
6. 新增独立 HTML 创作指导和自行编写的完整行为样例，覆盖搜索与分类组合、选中状态、结果播报、空状态、重置、原生键盘操作与响应式布局。保存成功只证明文件已保存，不能据此宣称浏览器交互已验收。

## 实际文件与验证边界

答辩修正版仅替换第 8 页公式块，保留原面板及三条公式含义。其余 13 页 slide XML、14 页 notes、图表和嵌入工作簿均与原件逐字节一致；生成器 lint 为空、OfficeCLI schema errors 为空。原附件 SHA-256 前后相同。

本机 WPS 12.1.25895 已实际显示独立原生公式样张，分式、上下标、帽号和求和可见，没有选择占位回退。解锁后又打开了实际 14 页修正版，第 8 页三条原生公式可见，并能进入原生公式工具。在独立副本 `WPS-公式编辑验收.pptx` 上执行保存、关闭标签，再经 Finder 精确选择同一副本重开，确认仍为同名 14 页文稿；第 8 页三式的帽号、分式、上下标、Greek 和求和下标均保留可见。本次通过的是 WPS 保存→关闭→重开后的显示往返；没有成功修改内部数学节点，公式内部编辑仍未验证。

副本 ZIP 的只读核对确认第 8 页仍有 3 个 OMML 数学对象，首式仍含等号而非约等号，全部 slide 的普通文本中无额外 `+0`。原附件及其归档副本 SHA-256 仍为 `b5f51200817c5767908bb0872847e437afc7e7b46606cd47f4929a2db125d093`，交付修正版仍为 `374940869c087817503b673d3673280167fab3b7e427f770c900a90650ba800a`；验收副本为 `94d3181c1a3096c5650782779ae0b7698bf4be9308a9b048e8bbbaceef0f4daf`。详细范围记录在私有 `wps-native-validation.json`。

实际修正版的第 8 页已通过原生文件 → 仅数学兼容副本 → OfficeCLI SVG → worker JPEG 链路。旧 worker 三处公式暗像素为 `[0,0,0]`；冻结适配器后三个区域合计 743 个暗像素，放大检查可读出三式，原生源文件哈希不变。另有独立生成样张回归要求每处最终 JPEG 暗像素至少 80；SVG 节点计数仅为结构证据。

景点 HTML 优化副本保留全部 12 个景点的 HTML 内容，新增搜索、计数、空结果与重置。真实浏览器已验证：全部 12 项 → 城市 2 项 → 不匹配搜索 0 项和空状态 → 重置 12 项，并把焦点恢复搜索框；Enter 可触发重置。375px、900px 的 DOM 宽度均无水平溢出，375px 截图确认筛选、焦点和卡片可读。未联网复核原旅游事实，本次保留原内容。

前端组件的 900px 明暗主题、分页、错误降级和键盘测试采用合成会话与 JPEG；它们不替代安装包媒体协议到真实附件的完整验证。独立复核已修正求和范围、phi 字形、ZIP 预检/头部校验和检查失败提示四项问题。前端三组件独立复核 510/510 通过；自编 HTML 示例的 375px/900px 浏览器检查也通过。完整回归结果见下方冻结记录；本轮本地替换和启动检查已通过，尚未从新安装应用发送生成请求。

## 公开依据

- 原生 Office 数学采用 [Microsoft DrawingML 扩展规范](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/853b19c7-68a9-4f9a-a2ae-5e6cb0d02e62) 的 `a14:m` 与兼容分支。PNG 回退遵循微软[公式与图片回退示例](https://learn.microsoft.com/en-us/openspecs/office_standards/ms-odrawxml/38b13e1f-1102-4bb7-819a-dd5d9abdb176)的 `p:sp/a:blipFill`。规范符合性不能证明 WPS/PowerPoint 实际显示和编辑。
- PNG 回退使用已打包的 [Matplotlib Mathtext](https://matplotlib.org/stable/users/explain/text/mathtext.html)；Mathtext 是 TeX 子集，不是完整 LaTeX。解析器的支持范围独立限定并测试，不能把 Mathtext 的所有语法宣称为支持。
- HTML 的可观察状态与键盘语义依据 [WAI 按钮模式](https://www.w3.org/WAI/ARIA/apg/patterns/button/) 和 [WCAG 状态消息说明](https://www.w3.org/WAI/WCAG22/Understanding/status-messages.html)。示例是自行编写的行为起点，不是要求所有页面使用同一视觉模板。

## 仍需验证

- WPS 第 8 页显示、进入原生公式工具，以及独立副本保存/关闭/重开后的显示已观察；公式内部编辑仍待验证。PowerPoint 与 Windows 分别验证。
- 新安装包的预览媒体协议、原生引擎与真实会话完整路径；旧附件没有新 JPEG 元数据时按无预览降级。
- 多个模型是否遵循新公式与交互指导，内容质量、视觉差异、耗时和实际费用是否改善；按既定统一样张条件隔离实验，先验证收益再扩库。
- 字体替换和 CJK 度量仍可能影响目标机器版式。有限 LaTeX 支持和兼容图片不承诺任意数学语法或完全一致的字体。

原评估器的 Rust 扫描路径已修正为实际 `src-tauri/src/services/assistant`，并增加必须包含 `documents/create.rs` 的回归。上一轮 180 文件快照漏记 Rust 的历史事实保留在[验证记录](2026-11-10-ppt-validation.md)，不得继续描述为完整生成链快照。

## 冻结版本检查记录

所有证据保存在 `.real-eval/ppt-wps-repair-2026-11-10/`。macOS arm64、基于 `2ebe6491` 的未提交工作树；Windows 原生执行未开展。最终助手 suite 606/606 通过、搜索 suite 102/102 通过。开发回归使用独立临时副本的 Node 22.23.4，避免已签名 staged Node 与开发原生 addon 的库验证冲突；未修改 staged/已安装 Node 的签名。安装包已另外使用真正签名的 Node 和 addon 完成离线验证，不能把开发套件当作打包验证。

历史失败日志保留：旧输出措辞断言、旧 mock 的非 ZIP 容器、规范回退的识别误报，以及仅计 SVG 节点而漏掉的空白 JPEG 均没有删除后重算通过率。实物权限链的首次隔离 fixture 还暴露 Homebrew Node launcher 缺少 libnode、以及借用图片记录却未填 PPTX metadata 的布置错误；仅修测试环境/夹具，不能放宽权限或安全规则来通过测试。后续发现模型首图和 contact sheet 仍直走旧 SVG 渲染，已在 `document-tools.mjs` 两处读取入口复用同一有界适配；两个独立模型 JPEG 用例先以 0 暗像素失败，再原样通过。最终版本无安装依赖、无真实模型请求、无提交或推送。

| 检查                                                  | 冻结结果                                                                | 本地证据                                                              |
| ----------------------------------------------------- | ----------------------------------------------------------------------- | --------------------------------------------------------------------- |
| TypeScript / 前端格式 / Rust fmt / all-targets Clippy | 通过                                                                    | `*-delivery.log`                                                      |
| 前端全量，`--maxWorkers=2`                            | 3111 通过、1 项按原配置跳过                                             | `frontend-full-delivery.log`                                          |
| Node 22 助手全量，`--test-concurrency=2`              | 606 通过、0 跳过                                                        | `assistant-full-delivery.log`                                         |
| 搜索模块                                              | 102 通过、0 跳过                                                        | `search-frozen.log`                                                   |
| Rust 全量                                             | 4429 通过、24 项显式 opt-in 忽略                                        | `rust-full-delivery.log`                                              |
| 额外实物 opt-in                                       | 数学兼容副本和沙箱渲染→缓存→受权协议均通过                              | `math-real-fixture.log` / `raster-protocol-frozen.log`                |
| 最新签名包                                            | 224 项资源哈希一致、5 个风格各 6 页、生产引擎 smoke 通过                | `packaged-runtime-delivery.log`                                       |
| 包内数学与模型预览                                    | native/PNG、strict sum、legacy warning、模型两预览像素回归通过          | `packaged-math-delivery.log` / `packaged-model-preview-delivery.json` |
| 本机备份、替换和启动                                  | 已安装；工具页账户/配置、助手历史、模型目录与输入框加载通过，未发送消息 | `installation-receipt.json` / `delivery-state.json`                   |

最新包使用与已安装应用相同的 bundle ID `ai.ofox.desktop`、版本 `1.5.0` 与签名 Team `R65NVCLBMY`。完整 release 构建成功后，模型预览的最后补改只影响外置运行时 JS，因此用官方 `tauri bundle --bundles app` 将最新已准备资源重新封装签名；包内全部 runtime/src、skills、引擎与 notices 再与源码核对，防止交付前一份资源。没有发布或改变版本号，未做公证。

**本轮修复已安装并通过启动检查；原生公式编辑验收仍待完成。** 此前 Mac 锁屏曾阻断替换与 WPS 操作，现已解锁。正常退出旧应用后，`replace-local-app.py` 以 exit 0 完成备份和替换，安装路径为 `/Applications/Ofox Desktop.app`，备份为 `$HOME/Library/Application Support/OfoxDesktopLocalBuilds/20261110-133405-math-ui`。

随后通过 CUA 从安装路径打开本轮应用：工具页账户与配置正常，助手页历史对话、DeepSeek V4.1 Flash 模型目录和输入框均加载完成；没有发送消息。只读进程检查确认运行路径为 `/Applications/Ofox Desktop.app/Contents/MacOS/ofox-desktop`，主程序 SHA-256 为 `bd8f60c1651b84e6ff436f8339969a8eb2f8ec7821a68961e3585e8921084489`，与本轮已核验构建一致。WPS 独立副本已通过保存/关闭/重开后的第 8 页三式显示验收；内部数学节点编辑仍未验证，PowerPoint/Windows 结论不变。没有修改原附件、交付修正版或旧聊天内容。

源码与包身份另存 `delivery-state.json`、`modified-code-snapshot.json`。195 个修改/新增代码文件围绕已知 HEAD 的集合摘要为 `b016fbfd50da6cf97b0794cee7e5823589f81c3e2a4a10c9b7e130f79251f52f`；它是 dirty 本地源码快照，不是 clean release SHA。修正 Rust 扫描后的生成快照包含 262 个文件，`generationSnapshotHash=86f9b00194e183040b22aec80ea71df24d3cdfd0779370398da1d0c1a4e0ee2a`；60 个模型 A/B/C/D 项仍为 pending、可比性 blocked，没有质量收益或费用结论。

## 后续真实 Transformer 验收

以上“未发送真实请求”“内部编辑未验证”保留为修复冻结时及旧 14 页样本的历史状态。随后用户授权新版应用真实生成 10 页 Cartesian Transformer：首轮约 15 分钟超时，恢复尝试于本地 14:40:05 完成，首轮失败保留。该新样本的独立副本已在 macOS WPS 12.1.25895 完成第 5 页主公式原生符号 A→B、保存、关闭、重开；这不替代旧样本、其他公式的结构编辑或 PowerPoint/Windows 验收。独立内容校正版的第 3/4/5/10 页也已在 WPS 实际复核，缺失字体提示仍在。详见[真实生成验收补录](2026-11-10-ppt-transformer-live-validation.md)，不据单例宣称完整质量提升。
