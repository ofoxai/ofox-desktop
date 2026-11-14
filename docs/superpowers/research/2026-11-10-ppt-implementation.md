# PPT 公开资源与原生风格首轮实施

日期：2026-11-10。工作树：`ofox-desktop-1.5.2`，基于 `feat/assistant-1.5.1@2ebe6491`，未提交的实现。原 [研究方案](2026-11-09-ppt-design-system-research.md) 保留方案修订时的源码诊断；当前状态以本记录、[资源入库说明](2026-11-10-ppt-resource-intake.md) 和 [验证记录](2026-11-10-ppt-validation.md) 为准。没有提交、推送或发布代码，没有新增依赖或替换引擎。后续按用户明确授权完成本机应用替换，状态见末节。

本页是首轮实现和首次本机替换的历史记录。后续实际文件的公式、分页预览与交互修复见[专项记录](2026-11-10-ppt-math-ui-repair.md)；本页旧验证表和固定样张不代表后续源码。评估器遗漏 Rust 扫描的修正也见专项记录与验证记录。

## 已落地的行为

- 筛选并保留 24 个公开候选身份：17 个 Beautiful HTML、6 个 OpenDesign `html-ppt`、1 个 OfficeCLI；143 份原始代码/文本按固定提交保留路径、SHA-256、MIT/Apache LICENSE 和 NOTICE。主题 CSS、组件、完整 deck、工作流和生成引擎分别计数。候选不是可用 PPTX 数量；闭源 ofox-agent 没有进入资源或实验内容。
- 五个代表 `blue-professional`、`cartesian`、`course-module`、`product-launch`、`architectural-plan` 可以实验选择，兼容默认 `navy`。另外 19 个仍为候选。`enabled` 表示具备可选择的适配器，`verified` 仍为 false，质量验收、字体和实际 PowerPoint 编辑各有独立状态。
- `get_presentation_style` 提供选择列表，以及来源版本、构图配方、原生覆盖范围、字体替代策略、素材边界和每页计划/成品检查依据。工具只读固定打包元数据，不执行上游 HTML/JS/Bash，也不接受来源路径。Python 和 Node 都只启用许可已审、已适配并明确 enabled 的主题。
- Python 使用 `new_deck(style=..., language=...)`，保留旧默认和模板兼容。五代表改变封面、章节、页头、字体角色和组件；`lesson_slide` 提供目标侧栏，`panel_slide` 提供 13cm 左右交替面板和编号，`slide_regions` 返回布局区。Node 回退同步风格选择、封面构图、教学侧栏和架构面板。
- `relation` 使用原生文字、形状和连接线；`equation` 支持文本、分式和上下标嵌套，生成 OMML Choice 与明确文本 Fallback。旧 `formula` 保留文本行为，`figure`/`plot` 保留 PNG 行为。没有把整页截图或任意 HTML/SVG 称为原生可编辑 PPTX。
- 风格选择、每页设计计划、创作指导与成品检查分开说明，并引用固定公开来源。原来强制填满内容区/固定卡片数的规则已放宽；连续比较可保持相同构图。现有预览、问题报告、重试限制和回合结束交付机制保留。

## 验证和独立审查

先取得缺失风格工具/选择、Python style 参数、OMML/关系图、lesson/panel 和评估脚本的失败回归，再实现并检查输出。独立审查发现并修复了流程箭头反向、多 deck 交错配色污染、蓝色比较标题低对比和图表系列同色、全局 vendor 忽略导致资源不能交付、评估快照不完整及共享指导污染 A/B 的问题。

离线统一内容已经生成兼容基线和五代表六份 PPTX，每份六页、16:9、必需事实无缺、lint 为空，OfficeCLI schema 校验错误数为零，包含原生 Chart/嵌入 XLSX/OMML 结构。成品和生成代码、helper、runner、资源、模板、引擎版本都有哈希绑定。它们是固定代码冒烟，不能作为模型 A/B/C/D 的生成质量或性能结果。

首轮 macOS arm64 最终检查（所有日志保留在忽略的 `.real-eval/`，不是提交资产）：

| 检查                                                 | 结果                                                      | 日志                                               |
| ---------------------------------------------------- | --------------------------------------------------------- | -------------------------------------------------- |
| `pnpm typecheck` / `pnpm format:check`               | 通过；新增 runtime/脚本/文档也通过定向 Prettier           | `ppt-typecheck-final.log` / `ppt-format-final.log` |
| `pnpm test:unit --maxWorkers=2`                      | 247 文件、3077 项通过；1 文件/1 项跳过                    | `ppt-vitest-retry2.log`                            |
| staged Node 全 runtime suite，`--test-concurrency=2` | 590 项通过，0 失败/跳过                                   | `ppt-assistant-tests-bounded.log`                  |
| 搜索服务 suite                                       | 102 项通过，0 失败/跳过                                   | `ppt-assistant-tests-final.log` 后半               |
| Rust fmt / `clippy --all-targets -- -D warnings`     | 通过                                                      | `ppt-rustfmt-final.log` / `ppt-clippy-final.log`   |
| `cargo test`                                         | 4419 项通过，0 失败，22 忽略（含明确付费/人工 opt-in 项） | `ppt-cargo-tests-final.log`                        |
| 独立复审                                             | 最终无新发现；19 项独立契约检查通过                       | 本次审查记录                                       |

首次前端默认并发有 5 项资源库用例超时/时序失败，限制并发后完整套件通过，没有修改前端断言。助手默认并发两次分别出现既有 MCP fixture readiness、进程回收断言不稳定；保留原日志后，使用相同断言和受限并发完整复跑通过，没有修改 MCP 代码或断言。新增技能长度、helper 签名、工具 schema 和四处真实 Host 工具清单契约失败已补齐，不以跳过处理。Rust 首次全量库测试通过，集成测试中缺新增工具名称的两项失败已修正，最终全量复跑已通过。

本页首轮固定样张的证据目录是 `.real-eval/ppt-validation-2026-11-10-accepted-snapshot/`。具体引擎/资源/技能哈希与全部首次失败见验证记录；旧目录保留历史，不把旧成品的复核当作当前版本。原生 Windows 未执行；macOS 检查不能替代它。

## 保留的限制与下一步门槛

1. 五套是代表性的部分原生适配，没有完整复制 HTML 的页面、动画、字体、图片和交互。原始字体未下载或嵌入，使用系统字体角色替代；图表/表格仍需更多风格映射。保留源构图特征不等于逐像素还原。
2. OfficeCLI 会把原生分式预览为 `180200`，缺少分数线。OMML 结构与 schema 通过不能抵消这个实际视觉缺陷。PowerPoint 16.109.1 已安装，但本次文件选择器自动化未完成载入，因此实际编辑、保存重开仍待验，不能据此判定文件兼容失败。
3. 真实模型四条件仍未执行。评估器冻结内容、版本和参数并记录收据；普通当前会话已包含新风格/逐页计划，会污染 A/B，所以 `comparability=blocked`。必须先建立条件专用的基线/技能/工具指导快照，再执行同模型、同内容的真实实验、盲审和编辑验收。没有读取钥匙串或调用真实 API，费用未知。
4. 通过事实、论证顺序、构图差异、字体溢出、实际对象编辑、耗时费用和目标平台验收，确认收益后再扩库。不能因为现有 24 个目录和五个选择项就把 PPT 质量优化整体标记 Done。

## 本机应用替换（用户后续明确授权）

- 用现有 `Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)` 构建 release `.app`，版本仍为 `1.5.0`，bundle ID 仍为 `ai.ofox.desktop`；目录名不冒充包版本。没有执行线上 release 脚本、提交或推送。
- 新包内的生产 Node/SDK/PDF/Office/JSONL/搜索/Typst 冒烟通过；五代表各六页、事实缺项和 lint 为零、原生 Chart/嵌入 XLSX/OMML 结构存在、OfficeCLI validate 错误为零。222 个资源/二进制哈希匹配；包内主程序、Node、OfficeCLI、Typst 和 macOS native addon 签名有效，同一 Team ID，保留最小 entitlements。全部使用临时 HOME，未读取真实凭据或调用真实模型。
- 旧应用、默认数据目录及路径配置完整备份到用户私有的 `~/Library/Application Support/OfoxDesktopLocalBuilds/20261110-015417/`；备份数据库 quick_check 为 `ok`。没有导出钥匙串，没有恢复、覆盖或清理用户数据。
- 先完整暂存并验证新包，再以目录重命名替换 `/Applications/Ofox Desktop.app`，失败时回滚；安装后严格签名与主程序/catalog 哈希再次通过，现有数据库仍与备份一致。旧原件已移到 Applications 之外的备份目录。
- 用户手动解锁后，已明确启动安装目录内的新应用：主窗口、原有对话列表和助手页正常显示，模型目录加载完成，消息输入可用；已确认真实进程路径及新主程序/catalog 哈希。没有发送聊天请求或执行付费模型验收。新旧本机包均为 Developer ID 签名但未公证，本次未关闭或绕过 Gatekeeper。

构建与安装收据：`.real-eval/ppt-local-install-2026-11-10/`，包内验证报告 `packaged-verification-1794247281290.json`。真实模型比较、PowerPoint 实际编辑与 Windows 的既有待验状态不因本机替换自动提升。
