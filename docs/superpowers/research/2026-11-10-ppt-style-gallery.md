# PPT 风格库上下文入口

本页记录初版两页样张入口的实现与安装。后续已扩展为八页自编原生示例，当前行为与新验收见 [全页预览记录](2026-11-10-ppt-full-preview.md)。

2026-11-10。用户在制作 PPT 时可以查看风格简介与样张，再确认选择；入口位于助手的“做 PPT”场景和输入框“＋”菜单，不新增全局导航。打开、搜索和预览使用随包静态资源，不发送消息、不请求模型、不产生模型费用。

## 选择与范围

画廊默认选中 Auto，确认后才进入 PPT 制作模式。确认选择会显示输入框风格 chip；已发送的用户消息显示不可交互的 `PPT · 风格名` 标记，保留原文。旧消息、未知或未开放的风格元数据不显示标记。可选项如下：

| ID                   | 用途                         |
| -------------------- | ---------------------------- |
| `auto`               | 由模型按主题、受众与密度选择 |
| `navy`               | 内置默认，通用汇报           |
| `blue-professional`  | 商务、咨询、经营复盘         |
| `cartesian`          | 学术、研究、论文介绍         |
| `course-module`      | 教学、技术培训               |
| `product-launch`     | 产品介绍与发布               |
| `architectural-plan` | 技术架构、方案咨询           |

初版 Auto 仅保留 PPT 意图。后续 v1 已实现确定性能力硬筛 → 模型设计判断 → Host 冻结并强制生效风格；最终选型包含模型判断，不能称为确定性最终选择。当前行为与真实验收见[文件工作区验收](2026-11-10-ppt-file-workspace-acceptance.md)。五种新增适配显示实验标识，不能据此声称通过任意内容或跨应用验收。资源目录共 24 项，其中 5 项已适配开放、19 项仍为候选；`navy` 是目录之外的原有默认。

## 数据流与约束

- `AssistantPresentationStyleId` 区分未启用、Auto 和六种手动风格。草稿按账户、权限代际和会话隔离，采用与技能草稿相同的内存保存方式；切换会话可恢复，应用重启不保存该草稿选择，也不建立全局默认。
- 发送及队列条目冻结选择，后续修改 chip 不影响已发送或排队的请求。原始用户消息以可选 JSON 字段保存选择，无数据库迁移。编辑默认继承原选择，仅显式替换时修改或清除；重试、重新生成继承原始用户消息。带风格的待发内容走普通发送或队列，避免只传文本的 steer 丢失选择。
- Host 在请求入场及分支继承后检查固定 ID 白名单，在取得请求密钥前拒绝未知选择。初版仅非 Auto 写入可信 `documents.selectedStyle`；v1 中 Auto 经候选匹配与设计判断冻结后也写入可信的 `effectiveStyle`，手选仍优先且生成中不可静默更换。
- Node 对 deck 对象覆盖 `theme`；手动风格拒绝无法可靠应用主题的原始 PPTX batch，要求使用 deck 或 Python 路径。Python 的 `--selected-style` 来自 Host 运行快照，模型工具参数不提供该选项；helper 覆盖 `new_deck` 风格，runner 在保存前检查匹配上下文。旧无工作流标记请求保持原行为；v1 Auto 冻结后同样使用可信 `effectiveStyle` 的主题及 Python 覆盖。

主要实现位于 `src/lib/assistantPresentationStyles.ts`、`PresentationStylePicker.tsx`、`AssistantPage.tsx`、`useAssistant.ts`、`useMessageQueue.ts`、Rust assistant 的 `types.rs` / `requests.rs` / `host.rs` / `documents/create.rs`，以及 runtime 的 `document-tools.mjs` 和 Python helper/runner。

## 初版样张来源

以下是初版两页缩略图的制作记录；后续八页自编原生示例与完整原模板是不同资源，现状见[全页预览记录](2026-11-10-ppt-full-preview.md)和[资源覆盖清单](2026-11-10-ppt-source-coverage-audit.md)。

六种风格各使用同一套独立自编中文内容，实际生成两页原生 PPTX（封面、内容），再用 Python helper → OfficeCLI SVG → JPEG 链路产生 12 张 960×540 样张。生成记录使用 Node v22.23.4、OfficeCLI 1.0.160；六份源 deck 的 lint 和 schema 检查均无报告项，检查记录确认正文为原生对象、未嵌入媒体。最终 12 张样张已经目视检查，未见裁切或重叠；这只证明这组样张。

图片总计 301,600 字节；生成过程未调用模型，Node 子进程网络被阻断，未改安装包或读取用户文件。未使用闭源代理内容、客户数据、上游示例正文、第三方图片或下载的字体。字体槽位可能由系统回退，未认证逐字形字体解析。来源、固定版本与 MIT / Apache-2.0 声明见 [样张 NOTICE](../../../src/assets/presentation-styles/NOTICE.md) 及相邻许可文件；样张是栅格展示，不是完整上游模板转换或可编辑模板文件。

本地生成与检查收据位于 `.real-eval/ppt-style-gallery-2026-11-10/preview-generation.json`、`preview-inspection.json`，包含源码、PPTX、图片哈希和检查范围。

## 已执行检查与待补验收

- hook / 队列聚焦测试 59/59 通过（其中 hook 19、既有队列 40）；消息组件 77/77 通过，包含手动 / Auto 标记、原文保持、未知 / 候选 / 旧消息隐藏。
- Node 文档相关聚焦测试 49/49 通过，含新增 3 项可信选择覆盖、Auto / 旧请求兼容、未知选择与手动 raw batch 拒绝；新增项先得到真实 RED，再转 GREEN。
- Rust `presentation_selection_` 聚焦测试由主代理串行执行：原先 2 项行为 RED，完成实现后 5/5 GREEN，覆盖原消息保存、未知 ID、分支继承、Auto / JSON 兼容和 Host → Python runner 参数。
- Python 可信覆盖专项 5/5 真实 Pyodide 用例通过；相关链路及历史标记经独立只读复核，无阻断项。当前类型检查、相关格式检查已通过。

最终源码检查：前端 3,192 通过、1 跳过；运行时 614 通过；Rust library 4,205 通过、23 忽略，全部 integration targets 227 通过、1 忽略。类型检查、生产源码及相关测试格式、Rust fmt 和 all-targets Clippy（warnings denied）均通过。首次 Host 预览测试因暂存运行器仍是中间版本失败；刷新 `assistant:prepare` 后，该项及全部 integration targets 已重跑通过，无需改动最终源码。

本地浏览器已检查 900×640 与默认窗口、浅色与深色、预览 Escape 返回、6 个样张卡片及当前选择说明；签名包已完成，Developer ID Team 为 `R65NVCLBMY`，未 notarize。包内 runtime / math / selection 三组离线探针均通过：五套适配各六页、原生数学与兼容回退，以及 Node 三例 / Python 两例的手选覆盖与 Auto 保持；固定源码哈希核对通过。Rust doc tests 另有 2 项保持 ignored、0 失败。

19:06 的首次安装检查因 Mac 锁屏暂停。用户随后解锁，确认旧应用空闲后正常退出，已完成 `/Applications/Ofox Desktop.app` 替换与原生启动验收；没有重新构建或调用模型。安装主程序 SHA-256 为 `c4f0ee88e191474d4014dd17c21ccf4df7a6c4d9c6bd109dbe52257b973af026`，运行进程路径已核对为安装目录。旧应用及数据快照保存在 `OfoxDesktopLocalBuilds/20261110-191358-ppt-style-gallery`；live 用户数据与 Keychain 未回写。

原生界面观察确认：历史会话和模型目录恢复，“做个 PPT”与输入框“＋ → PPT 风格”均打开选择器，列出 Auto 和六种风格；封面样张及 Cartesian 内容页预览显示正常，预览内确认后显示 `PPT · Cartesian` 且焦点回到仍为空白的输入框。临时改选 Auto 再取消后，Cartesian 保留。最后清除了测试选择，留下未确认 Auto 的风格库界面，未发送消息、未开始生成。安装双份收据、原生 UI 检查项与截图见 `.real-eval/ppt-style-gallery-2026-11-10/installation-status.json`、`installation-receipt.json`、`native-ui-verification.json`、`native-gallery.png` 和 `native-cartesian-chip.png`。本轮未重新执行“模型端到端生成整份演示”的验收，也未验证 Windows UI；样张生成不代表 WPS / PowerPoint 内部编辑、保存重开或任意内容的视觉质量已通过。
