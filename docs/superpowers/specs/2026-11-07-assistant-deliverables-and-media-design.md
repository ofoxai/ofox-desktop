# 助手：产出文件、媒体质量、内置技能

日期：2026-11-07
状态：已批准（2026-11-07，用户授权按本文建议做决定）
工作区：`/Users/yunshen/code/ofox-desktop-1.5.2`，分支 `feat/assistant-deliverables`，从 `feat/assistant-1.5.1`（700d0870）开出。

## 为什么做

助手现在能写内容，不能交付文件。用户说"写一个 pdf，介绍我的家乡"，它把作文写在聊天里，最后一句是"我这次没能直接附上 PDF 文件"；说"做一个 Transformer 介绍的 ppt"，它给出 8 页内容让用户"搬进 PowerPoint"。原因很简单：运行时只能读 Office 和 PDF（yauzl + saxes + pdfjs），没有任何写文件的工具；宿主的 `generated()` 只收图片；前端没有文档卡片。

图像和视频生成是"基本能用"：提示词原样透传，给模型的指引写着不要添加用户没说的内容；模型生成完拿到的只有文件 id，看不到自己生成的图，没法判断好坏再迭代；图片参考图只能 1 张；没有简报，没有变体对比。

技能和 MCP 的基础接线 1.5.1 已经做了（资源库分发、`/` 选技能、`use_skill`、`find_skills`、`install_skill`、`save_skill`、MCP deferred 接入 + 审批）。这一轮补的是上层：有什么技能值得随包内置。

另外，网关的 OpenAI 路由在并行工具调用时给每个调用的 `index` 都是 0（两个模型都复现），Pi 按 `index` 聚合会把两个调用并成一个，参数变成 `{"city":"Tokyo"}{"city":"Osaka"}`。这一轮先规避。

## 目标

**产出文件**
- 用户要 PDF、Word、PPT、Excel 时，助手生成真实文件，对话里出现文件卡片，能保存、能打开。
- 文件由确定性引擎生成：Office 三种格式用 officecli，PDF 用 Typst。模型只写内容和结构，不手写文件格式。
- 生成后有校验和预览反馈给模型：结构问题（`validate`、`view issues`）和第一页缩略图（支持视觉的模型），模型能自查再改。
- 能修改用户上传的 docx / pptx / xlsx，产出新文件。

**媒体质量**
- 模型能看到自己生成的图片（缩略图回传），能自查、能迭代。
- 提示词策略从"一字不改"改为"保主体、补技术描述、缺关键参数就问一句"。
- 图片参考图从 1 张放开到多张（取决于网关）。
- 接上已经写好但没接的"用作输入"和"重新生成"，重新生成时可以换参考图。

**内置技能**
- 办公：docx / pptx / xlsx / pdf 各一个撰写规范技能；前三个改编自 OfficeCLI 仓库的技能（Apache-2.0），pdf 是自写的 Typst 规范。
- 媒体：改编自 ofoxai/skills（MIT）的场景技能：广告片、产品视频、短剧、产品图、改图、首尾帧动画，共用一份提示词结构和创意简报参考。
- 随包技能由宿主并入 1.5.1 的技能清单，`use_skill` 能读，只读。

**网关并行 bug 规避**
- 请求里带 `parallel_tool_calls: false`，网关修好后移除。

## 不做的（这一轮）

- 技能脚本的独立沙盒和自带 Python。1.5.1 推迟到这一轮，但这一轮的文件产出不需要脚本（模型只写内容，引擎由宿主调），所以继续推迟。
- 需要浏览器登录（OAuth）的 MCP、MCP 常驻进程。
- 视频生成结果回传给模型（视频是异步完成的，模型不在回路里）。
- PDF 的编辑（只生成）；`.doc` / `.ppt` 旧格式；文档模板库；多人协作。
- 图片生成的费用确认（沿用现状：图片不弹，视频弹）。
- 在聊天之外的独立"创作面板"。
- Linux。

## 约束

- **不影响正在发布的 1.5.0 和在途的 1.5.1。**
  - 只在 `/Users/yunshen/code/ofox-desktop-1.5.2` 工作，分支 `feat/assistant-deliverables`。
  - 不 checkout、不修改、不推送 `release/1.5.0`、`feat/assistant-1.5.1`、`feat/assistant-media-agent`。
  - 1.5.1 视为冻结的基线：它若有新提交，用 `git merge` 合进本分支，不 rebase；技能清单相关的改动单独提交，便于解决冲突（我的决定）。
  - 不打 tag。每个任务完成后本地提交；不推送、不开 PR（用户全局规则：只 commit 不 push）。
- **磁盘和内存。** 开工前、每次 Rust 全量编译或 `pnpm tauri build` 前，先 `df -h /System/Volumes/Data`。可用空间低于 20 GB 时只能删 `/Users/yunshen/code/ofox-desktop/src-tauri/target`、`/Users/yunshen/code/ofox-desktop-assistant-media/src-tauri/target` 和本工作区的 `src-tauri/target/debug/incremental`，删完再查。同一时间只跑一个重任务（`cargo build/test/clippy`、`pnpm test:unit`、`pnpm assistant:test`、`pnpm tauri build` 不并行）。不要在 `/Applications` 安装或启动新版 app。
- **开源边界。** 本仓库开源。officecli（Apache-2.0，iOfficeAI）和 Typst（Apache-2.0）可以随包分发，必须把它们的 LICENSE 和 NOTICE 放进 `assistant-runtime/notices/`（打包测试会检查 notices 覆盖了实际打包的东西）。OfficeCLI 仓库的技能（Apache-2.0）和 ofoxai/skills（MIT）可以改编，保留来源说明。不拿 Anthropic 的 pdf / docx / pptx / xlsx 技能。提交信息和注释里不提闭源产品。
- **sidecar 的限制。** 运行时只有自带的 Node 22，没有 npm、没有 Python；生产依赖在构建时 `npm ci --omit=dev --ignore-scripts` 装进 `resources/runtime/`，不能依赖 postinstall。运行时到宿主（工具参数）每帧 ≤ 8 MiB，宿主到运行时（工具结果）每行 ≤ 1 MiB。工具调用上限：本机配置自 1.5.0 起每个对话默认开启，实际生效的是 60 步 / 15 分钟 / $1 加重复调用检测（`local-task.mjs`）；只有没有本机工具的运行才是 20 次。
- **附件上限。** 单文件 25 MiB、每会话 500 MiB（`attachments.rs` 的 `MAX_FILE_BYTES`）。产出的文档沿用这个上限。
- **签名。** macOS 上每个 Mach-O 都要签名（hardened runtime、时间戳），officecli 和 typst 和 Node 一样处理。发布平台是 macOS arm64 和 Windows x64。
- **界面文字三种语言**，放 `src/i18n/locales/{zh,en,ja}.json`。
- **Ofox 地址按区域生成**，用 `src/lib/ofoxUrls.ts` + `useOfoxApex`。
- **Windows 和 macOS 行为一致**，路径用平台 API 拼接。
- **项目规范。** pnpm；Conventional Commits，不带生成信息；每个提交可编译、测试全过；Rust 过 `cargo fmt --check` 和 `cargo clippy --all-targets -- -D warnings`；运行时 `src` 改动后先 `pnpm assistant:prepare` 再 `pnpm assistant:test`。
- **密钥。** 新工具的参数、结果、日志、事件沿用现有的 `scrubSecrets` / `redact`。officecli 和 typst 子进程用清洗过的环境变量，不继承 app 进程的凭据。

## 引擎与参考（哪些是现成的，哪些要自己写）

| 领域 | 现成的 | 怎么用 | 要自己写的 |
|---|---|---|---|
| DOCX / PPTX / XLSX | **officecli** v1.0.160（Apache-2.0，C# 单文件，mac-arm64 33.9 MB、win-x64 33.5 MB，发布包带 `SHA256SUMS`） | 随包分发，宿主按次 spawn，一份文档一次 `batch` | 工具、宿主调用、卡片、技能改编 |
| PDF | **Typst** v0.15.1（Apache-2.0，Rust 单文件，mac-arm64 压缩包 14 MB、win-x64 22 MB） | 随包分发；模型写 Typst 正文，宿主加页面和字体前言后编译；`--format png` 直接出预览 | 前言模板、字体回退、工具 |
| 文档预览 | 已打包的 pdfjs + `@napi-rs/canvas`；Typst 自带 PNG 导出；officecli `view html` / `view svg` 不需要浏览器 | PDF 用 Typst 的 PNG；PPTX 用 `view svg` 再栅格化（待验证） | SVG 栅格化 |
| 媒体提示词 | **ofoxai/skills**（MIT）：`ofox-video-core/references/prompt-structure.md`、`creative-brief.md` 和 6 个场景技能 | 改编成随包技能，把 CLI 调用换成 `generate_image` / `generate_video` | 改编、图片侧的提示词规范 |
| 办公撰写规范 | **OfficeCLI 仓库 `skills/`**（Apache-2.0）：word / pptx / excel / pitch-deck / financial-model / data-dashboard | 改编成随包技能，去掉 shell，改为 `create_document` 的 batch | 改编、pdf 的 Typst 规范 |
| Harness | Pi 1.5.0（已在用）：调用上限、unknown-outcome 门、压缩、重复检测 | 不动 | 并行 bug 规避一行 |

没有现成可接的：图片提示词增强没有像 officecli 这样完备的开源组件，靠技能和缩略图回传；PDF 导出 officecli 设计成插件但官方没发布，所以用 Typst。

## 第一部分：产出文件

### A1. 引擎打包

- `scripts/assistant-prepare.mjs` 增加两步，做法和下载 Node 一样：按 `assistant-runtime/engine-downloads.json` 里记录的版本、URL 和 SHA-256 下载 officecli 和 typst 的当前平台二进制，缓存在 `.cache/`，校验不过就拒绝；解压后放到 `resources/engines/officecli/` 和 `resources/engines/typst/`。`resources/manifest.json` 记录两者的版本。
- macOS 签名流程把这两个二进制和 Node 一并签（`APPLE_SIGNING_IDENTITY` 时 hardened runtime + 时间戳）。
- `notices/` 加 `officecli.txt`（LICENSE + NOTICE + THIRD-PARTY-NOTICES）和 `typst.txt`。`packaging.test` 覆盖新文件。
- 宿主通过 `resources/manifest.json` 找到可执行文件路径，和找 Node 的方式一致。缺文件时文档工具不注册，系统提示告诉模型"本机无法产出文件"，UI 不显示生成入口。

### A2. 模型看到的工具

**`create_document`**（运行时注册，经 `hostTool` 交给宿主）

```
{
  name: string,                 // 文件名，含扩展名；宿主做清洗
  format: "pptx" | "docx" | "xlsx" | "pdf" | "md" | "txt" | "csv" | "html" | "json",
  content: string | object[],   // 见下
}
```

| format | content | 宿主做什么 |
|---|---|---|
| pptx / docx / xlsx | officecli `batch` 的命令数组（和 `officecli batch --input` 的 JSON 一样） | `officecli create` → `officecli batch --input --json`（默认原子，失败整体回滚）→ `validate` → `view issues --limit 20 --json` |
| pdf | Typst 正文（不含页面设置和字体） | 把宿主的前言模板和正文拼成 `main.typ` → `typst compile` 出 PDF → `typst compile --format png --pages 1` 出预览 |
| md / txt / csv / html / json | 文本 | 直接写文件 |

- 一次调用产出一个文件；一条回复最多 3 个文件；`content` 序列化后 ≤ 2 MiB（远低于运行时到宿主 8 MiB 的帧上限）；返回给模型的结果连同缩略图 < 1 MiB（宿主到运行时的行上限）。
- **各格式支持的命令子集**（随包技能只教这些，测试矩阵也只覆盖这些；超出的命令 officecli 照样能执行，但不保证）：
  - docx：标题（Heading1–3）、段落、有序/无序列表、表格、图片、分页、粗体/斜体/字号/颜色、页眉页脚。
  - pptx：新建幻灯片、标题、文本框（要点列表）、图片、表格、备注、背景色、简单形状。
  - xlsx：单元格值与公式、粗体/颜色/对齐、列宽、合并、冻结首行、简单图表（柱/折线/饼）。
  - 使用某格式前必须先 `use_skill` 对应的办公技能（技能清单在系统提示里，模型知道它们存在）；假模型 e2e 要包含这次 `use_skill` 调用。
- **Office 内容由宿主按白名单校验，通过才交给 officecli**（"超出的命令 officecli 照样能执行"只对白名单内成立）：命令只有 `add`、`set`、`remove`、`move`、`swap`，字段只认 officecli 批处理的那几个；元素路径必须从已知根开始（`/body`、`/slide[n]`、本批次自己加的工作表等），图示（diagram、flowchart）、OLE 和嵌入对象不能加（图表仍可）；属性值按键的类别逐项规则校验——文件类属性（`src`、`image` 等）只接受 `data:` URI（本地路径和远程地址一律拒绝，所以图片只能内嵌），链接只接受 `http(s):`、`mailto:` 或幻灯片，正文类属性（`text`、`notes` 等）不限，其余属性值不得指向文件或地址；Word 域只允许页码和日期类（PAGE、NUMPAGES、DATE、TIME、TOC）。拒绝时给出哪一项、怎么改，不产出任何文件。实现与取舍见 `docs/assistant-engines.md` 和 `documents/create.rs` 的 `check_batch`。
- 返回给模型：

```
{
  status: "completed",
  file: { id, name, mimeType, bytes, pages? | slides? | sheets? },
  issues: [{ severity, message, path? }],   // 来自 validate / view issues / typst 的警告，最多 20 条
  preview: image                             // 第一页 / 第一张幻灯片的缩略图，只给支持视觉的模型
}
```

失败时返回 `{status:"failed", error:{code, message, items?}}`：officecli 的逐项错误（`index`、`code`、`output`）和 Typst 的编译错误（含行号）原样回传，模型可以修正后再调一次。指引里写明：同一个文件最多重试 2 次，之后向用户说明。

**`inspect_document`**（只读，任何模式不弹审批）

```
{
  attachment_id: string,        // 用户上传的 docx/pptx/xlsx，或本会话生成过的文档
  path?: string,                // 不填为 "/"
  depth?: number,               // 默认 2，上限 4
}
```

宿主执行 `officecli view <file> outline --json` 和 `officecli get <file> <path> --depth N --json`，结果截断到 64 KiB 后回传。模型用它拿到段落、幻灯片、单元格的路径（含稳定 id 形式 `/body/p[@paraId=…]`），再写 `edit_document` 的命令。`read_attachment` 仍然只给纯文本，两者不合并。

**`edit_document`**

```
{
  attachment_id: string,        // 用户上传的 docx/pptx/xlsx，或本会话生成过的文档
  name?: string,                // 不填沿用原名
  content: object[],            // officecli batch 命令数组，路径来自 inspect_document
}
```

宿主把源文件复制到产出目录，执行 batch，校验，作为**新的**生成文件导入；源文件不动。只支持三种 Office 格式。

**系统提示**
- 现有系统提示里没有禁止产出文件的话；模型说"没能附上"只是因为没有工具。现有的"不要声称读过访问不到的文件"那句保留。新增指引：用户要文件、要"写一份 / 做一个 PDF、Word、PPT、Excel"时，用 `create_document` 产出文件；不要说无法附上；回复里先用两三句话说明文件结构，再调用工具，拿到结果后简述内容并指出 `issues`。
- `create_document` 的 `promptGuidelines` 只有几行：做什么、`content` 的形状、"第一次产出某种格式前先 `use_skill` 对应的办公技能"。命令参考全部放在技能正文里（见 A5，`use_skill` 上限 60000 字），不常驻系统提示。

### A3. 执行位置与数据流

```
模型 ──create_document──▶ 运行时（校验参数、scrubSecrets）
                              │ hostTool
                              ▼
宿主 dispatch_document_tool
  1. 产出目录 `<sid>/outputs/<docId>/`（0700；不放进运行时自己推导的 workspace/，宿主用 `AssistantStore::outputs_dir`（内部是 `private_subdir`）创建；运行时在同一次工具调用里读预览，所以目录跨过这次调用；run 结束时宿主按 `<docId>` 逐个删除，尽力而为，失败只记日志，不让 run 失败）
  2. 写入 content（batch.json 或 main.typ）
  3. spawn 引擎：清洗后的环境变量，HOME 和 TMPDIR 指向 outputs/<docId>/.home（officecli
     会写 ~/.officecli/config.json，batch 输出超过 8 KB 会落临时文件）；
     OFFICECLI_NO_AUTO_RESIDENT=1、OFFICECLI_SKIP_UPDATE=1（禁止后台检查更新）；
     typst 用 --root <outputs/docId> 和空的包目录（禁止联网取包）；
     超时 60 s；stdout/stderr 各 ≤ 256 KiB；进程组/JobObject 回收
  4. 校验：magic（%PDF、PK）、大小 ≤ 25 MiB、扩展名与 format 一致
  5. attachments::generated_file() → Attachment{origin:"generated", kind:"document"}
  6. 发 generation 事件 {messageId, generation:{kind:"document", …}, attachments}
  7. 结果里带文件的绝对路径和（pdf）typst 出的第一页 PNG 路径——只给运行时，不给模型
  8. 运行时出缩略图（它有 @napi-rs/canvas 和 pdfjs，宿主没有图像库）：pdf 读 typst 的 PNG；
     pptx 读 officecli view svg --start 1 的 SVG 栅格化（待验证，不行就不给预览）；
     长边 768 px、JPEG ≤ 200 KB；超预算就降质，再超就不附
  9. 运行时把 preview 作为图片内容块放进工具结果（只对支持视觉的模型）
```

- **权限**：产出只写 `outputs/`，不碰用户文件系统，任何权限模式都不弹审批；计入每条回复的工具调用次数（普通对话 20 次）。
- **幂等**：沿用 `toolCallId` + 参数指纹去重。
- **unknown-outcome**：子进程被中止时按现有规则处理。

### A4. 宿主与前端

**宿主**
- `attachments::generated_file(bytes, name, mime)`：按魔数和扩展名校验 mime；`media_protocol::open_file` 的白名单加 pdf / docx / pptx / xlsx / md / txt / csv / html / json（保存走已有的 `assistant_save_media`，它接受任意附件 id）。
- 不做"打开私有副本"：用户在 Word 里编辑一份会被清理的临时副本是陷阱。改为 `assistant_save_media` 增加 `open: bool` 参数：保存对话框 → 原子写入用户选的位置 → `open == true` 时用 `tauri-plugin-opener` 打开刚保存的文件（我的决定）。不新增 exports 目录，不暴露会话私有目录。
- `MediaRecord`/`AssistantGeneration` 加 `kind:"document"`，记录 `format`、`bytes`、`pages/slides/sheets`、`issues` 数量；文档不计费，`price_snapshot` 为空。

**前端**
- `GenerationCards` 加 `DocumentResult` 分支：类型图标、文件名、大小、页数/张数/表数、`issues` 徽标（点开列出）；按钮：**保存并打开**（主按钮）、**保存…**。
- `ReplyMeta` 的生成费用行按 kind 显示，文档显示"文档 ×N"，不显示金额。
- `AssistantMessageView` 过滤 generated 附件的逻辑对文档同样生效。
- 空对话 starters 加一个"做一份 PPT"（我的决定，可裁剪）。
- 文案三语。

### A5. 随包办公技能

- 位置：`assistant-runtime/skills/office-docx/`、`office-pptx/`、`office-xlsx/`、`office-pdf/`，各一个 `SKILL.md`，可带 `references/`。打包脚本已经整目录拷贝 `skills/`。
- 内容：改编 OfficeCLI 仓库的 word / pptx / excel 技能（Apache-2.0，`notices/` 注明来源）：保留版式规则（标题层级、字号、配色、每页要点数、表格样式、常见错误），把"运行 officecli 命令"改成"产出 `create_document` 的 batch 数组"，去掉 shell、resident、watch 等内容。`office-pdf` 是我们自己写的 Typst 规范：只写正文，标题用 `=`、`==`，表格、列表、图片的写法，中文排版注意事项，不用外部包。
- **怎么进清单**（我的决定）：
  - 宿主组装 1.5.1 的 `params.skills` 时，追加 `runtime/skills/*/SKILL.md`（排除 local-config，它另有通道），标记 `bundled: true`，档位固定"自动用"，不可卸载、不可写。宿主侧 `use_skill` 对 `bundled` 条目直接读随包目录，不走资源库 DB 和 proof。
  - 随包技能的 `name` 一律带 `ofox-` 前缀（`ofox-office-pptx`、`ofox-media-core`…），避免和用户技能重名；仍然重名时用户技能优先，随包的那条不进清单并记日志（运行时 `replaceSkillIndex` 对重名是两条都丢，宿主要在组装时先去重）。
  - 这一轮只在助手里可见：`/` 列表（`assistant_list_skills` 合并随包条目）和系统提示；**不在资源库技能页展示**，不碰资源库 UI 和同步逻辑。
  - 共用参考不用跨目录读：`ofox-media-core` 本身是一个带 `SKILL.md` 的技能，场景技能在开头写"先 `use_skill` ofox-media-core"。随包技能目录不进 `Scope.readable`，模型用不到 `read`。
- 办公技能的描述要短，四个加起来不超过 1.5.1 规定的 8000 字清单预算的一小部分。

### A6. PDF 前言模板与字体

- 宿主维护 `templates/pdf-prelude.typ`：A4、页边距、正文字号、标题样式、页码；字体回退链 `("PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Hiragino Sans", "Yu Gothic", "Helvetica Neue", "Arial")`。Typst 默认搜索系统字体，macOS 和 Windows 都有中文系统字体，这一轮不打包字体（我的决定；如果验收时发现某个平台缺字体，再打包 Noto Sans CJK SC，OFL，约 10–16 MB）。
- 模板用 `#set text(lang: "zh")` 按界面语言设置。
- 模型写的正文里引用图片：允许 `image("assets/<attachment_id>")` 形式，宿主在编译前把本会话该附件复制到 `outputs/<docId>/assets/` 并补上扩展名（M3 做）。

## 第二部分：媒体质量

### B1. 模型能看到生成结果

- `generate_image` 的工具结果里附缩略图：对 `model.supportsImages` 的模型，每张图附一张长边 768 px、JPEG ≤ 200 KB 的缩略图，作为工具结果里的图片内容块（Pi 的工具结果支持图片）；文本部分不变。一条结果最多 4 张，总帧 < 1 MiB。
- 不支持视觉的模型：不附，文本里注明"你看不到图片，请让用户判断"。
- 改图（带 `image_ids`）同样回传。

### B2. 提示词策略

把 `media-tools.mjs` 的指引从"不要添加用户没说的主体、风格和细节"改为：

- **保主体、保意图**：不添加用户没提的主体、文字、品牌、人物；不改变用户明确指定的风格。
- **补技术描述**：构图、景别、光线、镜头、材质、色调、画幅，用生成模型喜欢的结构化写法（随包技能里给模板）。
- **缺关键参数就问一句**：用途、画幅/比例、视频时长和有无人声，不清楚时在生成前问**一个**问题；用户说"直接出"就用默认值。
- **和 composer 生成开关的优先级**：用户在 composer 里打开了图片/视频生成 chip，说明参数已经在 `MediaDefaults` 里定了，**不问，直接用默认值生成**；只有没开 chip、又说不清用途或画幅时才问。现有指引"开关打开时一描述就调工具"保持不变。
- **透明**：回复里给出最终使用的提示词，用户可以让它照原话改。
- **用户说"按我写的来"就一字不改。**

### B3. 多参考图

- `generate_image` 的 `image_ids` 上限从 1 放到 4，**只对允许多图的模型**：目录没有这个字段，宿主维护一张按模型族的表（和现有 gpt-image-2 的尺寸兜底表放一起），默认 1；探测通过的模型族填 4（我的决定）。运行时的 `maxItems` 跟着宿主传来的上限走。
- 费用估算：按张计价的模型不变；按 token 计价的模型把输入图 token 乘以张数（`media.rs::cost()`）。
- M1 先探测网关 `/v1/images/edits` 是否接受多个 `image[]`（用本机 key 的探测脚本，不进上下文）。不支持就全部保持 1，技能文案一致。`RegenerateDialog` 的参考图选择器按同一上限限制。

### B4. 变体与迭代

- 接上 `MediaBlock` 已有的 `onUseInput`：把这张图加入 composer 的附件，模型下一轮就能用 `image_ids` 引用。
- 接上 `onRegenerate`：`RegenerateDialog` 增加"更换参考图"（从本会话附件和生成图里选）。
- 默认张数不变（1）。随包技能建议：用户在探索风格时提议"出 2 张供选择"并说明费用。

### B5. 视频流程

- 随包技能教模型：先用 `generate_image` 出首帧（和可选的尾帧），确认后再用 `generate_video` 的 `material_use: "frames"`；时长、画幅、人声通过简报确定；费用确认沿用现有的 `confirm_generation`。
- 不改视频的请求链路。

### B6. 随包媒体技能

- 位置：`assistant-runtime/skills/ofox-media-core/`（一个真正的技能，`SKILL.md` 正文就是提示词结构和创意简报，改编自 ofoxai/skills 的 `ofox-video-core/references/prompt-structure.md` 和 `creative-brief.md`）、`ofox-media-video-ad/`、`ofox-media-product-video/`、`ofox-media-short-drama/`、`ofox-media-product-image/`、`ofox-media-image-edit/`、`ofox-media-keyframe/`。场景技能开头写"先 `use_skill` ofox-media-core"。
- 改编规则：保留简报问题集、提示词公式、时间戳分段、镜头词汇、一致性锁和负面清单；把 `ofox-video.sh` / `ofox-image` 的 CLI 流程换成 `generate_image` / `generate_video` 工具调用；去掉 `OFOX_API_KEY` 相关内容（助手用自己的 key）；费用说明改为引用工具返回的 `estimatedCostUsd`。
- `notices/` 注明来源（MIT，ofoxai）。

## 第三部分：网关并行工具调用 bug 规避

- 只对 **OpenAI chat-completions 形状**的请求加 `parallel_tool_calls: false`（在现有包装 `streamSimple` / `completeSimple` 的那一层按请求形状判断，实现者定具体位置）。Responses 和 Anthropic 路由不加：该字段只属于 chat-completions 格式，塞进其他路由可能 400。bug 实测只在 `/v1/chat/completions`（`openai/gpt-6.1-terra`、`openai/gpt-6.1-sol` 都复现）；M1 的探测脚本顺带测一次 Responses 路由，有同样问题再单独处理。
- 测试：假模型服务断言 chat-completions 请求体含该字段、Anthropic 形状的请求体不含。
- 代码注释写明原因和移除条件：网关 SSE 的 `delta.tool_calls[].index` 对第二个调用不递增。

## 最容易出问题的地方（每条都要有测试）

1. **模型还是把内容写在聊天里而不调工具**：假模型 e2e 断言"写一个 pdf"的回复包含 `create_document` 调用；系统提示的新措辞要被测试钉住。
2. **batch 内容坏掉**：非法 JSON、不存在的命令、路径错误。officecli 原子回滚，宿主把逐项错误原样回传，不留半成品；超过 2 次重试后模型向用户说明。
3. **Typst 编译失败**：语法错误、引用不存在的图片、字体缺失。错误原样回传；字体缺失只警告不失败（回退链）。
4. **引擎缺失或签名坏了**：`resources/engines/` 缺文件时工具不注册、UI 不显示入口、系统提示说明；macOS 上 `assistant:smoke` 要实际运行一次 officecli 和 typst。
5. **缩略图撑爆帧**：缩略图编码后超过预算就降质到 ≤ 200 KB，再超就不附；整帧 < 1 MiB 有测试。
6. **"保存并打开"打开的必须是用户保存的那份**：测试断言打开的路径等于保存对话框返回的路径，且不在会话目录内。
7. **多参考图网关不支持**：探测失败时 `image_ids` 保持上限 1，技能文案一致。
8. **officecli 在 hardened runtime 下跑不起来**：它是自包含、trimmed 的 .NET 10（非 NativeAOT）。.NET JIT 在 hardened runtime 下需要 `com.apple.security.cs.allow-jit`。M1 的第一项任务是签名 spike：用 Developer ID 签 officecli 和 typst，跑 `assistant:smoke`，并走一次公证。**修订（R16）：entitlements 收窄为 `allow-jit` 一项。** 原计划按 Node 的做法给 officecli 同样三项（`allow-jit`、`allow-unsigned-executable-memory`、`disable-library-validation`），spike 的实测不支持：上游用 Developer ID 签的 v1.0.160 macOS arm64 版本就只带 `allow-jit` 并能运行；单文件包里是 59 个托管程序集、没有原生二进制，运行时不解压，`otool -L` 只列出 Apple 系统库，没有别的团队的库需要库验证放行；只带 `allow-jit` 签名后，所有 officecli 操作都通过。`disable-library-validation` 会拿掉"进程能加载哪些库"的检查，而这个进程读的是别人给助手的文档，所以不给。只在签名后的二进制出现被杀、`Code Signature Invalid`、库加载被拒等实际失败时，才按 `docs/assistant-engines.md` 的"When the signed engine fails"逐步加（先 `allow-unsigned-executable-memory`，再 `disable-library-validation`，再 `allow-dyld-environment-variables`）。公证还没有跑（需要发布负责人的 Apple 凭据），见同一文档。
9. **引擎偷偷联网或写用户目录**：`OFFICECLI_SKIP_UPDATE=1`、HOME/TMPDIR 指向产出目录；测试断言子进程环境里有这两项且 HOME 不是用户主目录。

## 已决定（用户授权后由本文作者决定）

1. **与在途 1.5.1 的合并策略**：1.5.1 视为冻结基线，不改它；有新提交就 merge 进来，不 rebase；技能清单改动单独提交。
2. **签名 spike 失败时的退路**：macOS 上 officecli 改为首次使用时按需安装，走现有 `install_tool` 流程（下载 `SHA256SUMS` 校验、放进 app 数据目录），三种 Office 格式都保留；Typst 是 Rust 二进制、不需要 JIT entitlements，PDF 不受影响。Windows 不受此影响，仍随包分发。

## 留给实现者决定

- 组件拆分、文件命名、Rust 模块放在 `services/assistant/documents/` 还是并入 `media_tools`。
- `pdf-prelude.typ` 的具体样式；Typst `--root` 和包目录的具体参数。
- SVG 栅格化的实现（`@napi-rs/canvas` 能否直接渲染 SVG；不能就不给 pptx 预览）；运行时缩略图模块的组织。
- `inspect_document` 结果的截断方式。
- 缩略图的编码参数；exports 目录的清理实现。
- 技能文案的具体措辞和长度分配。
- `parallel_tool_calls` 注入的具体位置。
- 探测脚本放 `scripts/` 的名字和参数。
- `engine-downloads.json` 的格式（可以和 `node-downloads.json` 一致）。

## 验收

- `pnpm typecheck`、`pnpm format:check`、`pnpm test:unit` 全过。
- `cargo fmt --check`、`cargo clippy --all-targets -- -D warnings`、`cargo test` 全过。
- `pnpm assistant:prepare` 下载并校验两个引擎；`pnpm assistant:test`、`pnpm assistant:smoke` 全过，smoke 实际运行 officecli 和 typst 各一次。
- 假模型 e2e（`src-tauri/tests/assistant_host.rs` 风格）：
  - "写一个 pdf 作文"→ 产出 PDF，卡片出现，`pages ≥ 1`，预览已回传；
  - "做一个 8 页的 ppt"→ 产出 PPTX，`slides == 8`，`validate` 通过；
  - "做一张成绩表"→ XLSX；
  - 上传一份 docx，"把标题改成 X"→ `edit_document` 产出新文件，源文件字节不变；
  - batch 非法 → 失败回传、无文件；
  - `generate_image` 结果含缩略图且帧 < 1 MiB；
  - 请求体含 `parallel_tool_calls: false`。
- 用户在本机用真实 key 跑：四种格式各一份中文文档，用 Word / PowerPoint / Excel / 预览打开无告警；生图一次，看到模型描述自己生成的图；用"用作输入"改一次图；视频走首帧流程一次。
- Windows x64 手动跑一遍上面四种格式，中文字体正常。

## 里程碑

| 里程碑 | 交付 | 验收 |
|---|---|---|
| M1 引擎 | 签名 spike（Developer ID + entitlements + 公证）→ `assistant-prepare` 下载校验打包 officecli + typst、notices、manifest；宿主能定位可执行文件；网关探测（多参考图、Responses 路由并行 bug） | smoke 跑通两个引擎；公证通过 |
| M2 产出文件 | `create_document` 四种格式 + 文本格式、校验、预览（运行时出缩略图）、文档卡片、保存并打开、系统提示改动、`ofox-office-*` 四个技能和随包技能进清单（M2 就要，因为产出前必须 `use_skill`） | e2e 前三条；技能在 `/` 列表里可见 |
| M3 编辑 | `inspect_document`、`edit_document`、PDF 内嵌图片 | e2e 第四条 |
| M4 媒体 | 缩略图回传、提示词策略、多参考图、用作输入 / 重新生成换参考图、六个随包媒体技能 | e2e 第六条；本机真实验收 |
| M5 收尾 | 并行 bug 规避、i18n、文档、Windows 验收 | 全部验收项 |
