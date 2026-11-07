# 助手待办清单（按优先级）

记录日期：2026-11-07。基线：分支 `feat/assistant-deliverables`（M1/M2 已完成：
PDF/PPTX/DOCX/XLSX 可稳定生成，引擎随包分发并签名，批处理沙箱与公式白名单）。
产品定位：助手直接产出**一份**用户要的、精美的 Office 文件，首轮即可用。

状态：`todo` 未开始 / `doing` 进行中 / `done` 完成。

## P0 主线：产出效果

### 1. Office 产出精美化（先 PPT，再 Word，最后 Excel） — todo

现状：版式由模型按坐标写，效果随模型波动；officecli 只能查溢出和出界，查到后回传模型重做，
一次请求出两份。

- 结构化中间格式：封面、要点、多卡片、KPI、表格、图表、结尾，约 7 种页型，模型只写内容。
- 两到三套设计主题，用母版、版式和占位符承载字体、配色、间距；代码把中间格式渲染成 officecli 批处理。
- 出卡片前自动质检：渲染全部页面，溢出、出界按建议直接修正，不再绕回模型。
- 一次请求只出一份文件；模型直接写原始批处理保留为专家模式和兜底。
- 验收：四个验收提示（PDF 四川、8 页 Transformer PPT、三科成绩表、Word 周报）在 terra、Luna、
  DeepSeek Flash 三个模型上各跑一次，一次出一份、零警告、三个模型观感一致。
- Word：样式模板（标题、正文、表格、页眉页脚）；Excel：表格样式、数字格式、图表样式。
- PDF 走 Typst 本就是代码排版，只补主题。

### 2. 在线预览，只生成一份 — todo

参考 ChatGPT 画布和 Codex 右侧预览：生成的文档在右侧面板直接翻页查看，不必先保存再打开。

- PPT 每页 SVG（宿主已能产出首页 SVG，扩展到全部页面）；PDF 用现有 pdf.js 渲染；
  Word、Excel 用 officecli `view html` 快照转图，或严格隔离后展示。
- 安全约束沿用：app 本身永不加载文档文件（媒体协议拒绝提供文档），预览只用宿主在沙箱里渲染出的图片或 SVG。
- 被新版本替代的旧卡片折叠显示为“已重做”，最终目标是同一请求只留一份（与第 1 项的质检闭环配合）。
- 预览面板内提供“保存并打开”“保存…”，与卡片一致；zh / en / ja 文案。

## P1：图像与视频

### 3. 图像生成补齐 GPT Image 2.6 能力 — todo

现状：对话里的图片生成只能选模型和张数（见 2026-11-07 截图）。网页版 ofox.design 已有参考图、
比例、模型切换、GIF 逐帧编辑；桌面端落后。网关侧进展见 Fizzy 卡片
https://app.fizzy.do/6179973/cards/1468（2026-11-06：除 markup 待复核外参数已透传，SVG 已支持且额外计费）。

- 模型：`openai/gpt-image-2.6-flare`、`openai/gpt-image-2.6-sunburst`，按发布时间排序展示。
- 参数：尺寸/比例、quality、background、output_format（png / jpeg / webp / gif / svg）、complexity。
- GIF：`frame_count` 2–16（默认 9）、`frame_duration_ms`；最小分辨率限制要在界面上提示。
- SVG：`svg.model` 会额外调用文本模型，产生两份费用，成本确认里要单独列出。
- 改图：`/v1/images/edits` 支持参考图和 markup 标注图；对话里支持拖入参考图。
- 错误：网关已统一包装报错，桌面端按新格式展示。
- 验收：与 ofox.design 网页版同一提示词、同一参数出图一致；成本显示与用量页一致。

### 4. 视频生成效果优化 — todo

现状（2026-11-06 用户反馈）：图像、视频生成质量差，主要是提示词和参数选择问题。

- 先定质量标准：挑 5 个典型场景（产品广告、电商展台、短剧对白、动漫分镜、讲解视频）各留一组参考样片。
- 把 seedance-* 系列技能里的提示词结构（分镜、时间戳、镜头语言、一致性锁）内置到助手的视频工具里，
  模型只填场景要素。
- 参数：首尾帧、参考图、比例、时长、分辨率；成本确认前给出预估。
- 生成中进度与失败原因可见；失败不重复扣费。

## P2：agent 侧加固与收尾

### 5. FullAuto 模式的围栏 — todo

只影响 FullAuto；Default 和 AutoEdit 下现有“审批 + 路径范围 + 加密日志还原”已够用。

- bash / PowerShell 的 OS 级沙箱（macOS 复用文档引擎的 `sandbox-exec` 方案；Windows 另评估）。
- 危险命令预检（`rm -rf`、`sudo`、`chmod 777` 等）直接转为询问，作为体验层而非安全层。
- 不持有 sudo；需要提权的动作走系统原生提权框，逐个确认。
- 若将来有定时或无人值守运行，所有询问一律视为拒绝。

### 6. 文档任务的模型路由 — todo

生成文件时自动选一个强模型，不跟随聊天模型；用户明确指定时以用户为准。

### 7. M1/M2 遗留 — todo

- `documents/create.rs` 拆分；`'x|y'!A1` 带引号工作表名的 `|`；`/Sheet1/cf[1]` 规范路径；
  `Book.xml!A1` 联合成员；PDF 页数伪造；崩溃残留输出清扫；一条偶发不稳定的运行时测试。
- `assistant-prepare.mjs` 的“是否最新”检查不认签名变化：签名构建后 `pnpm assistant:test` 会失败，
  目前靠 `rm -rf assistant-runtime/resources/node && pnpm assistant:prepare` 恢复。
- DMG 打包在 2026-11-07 本机构建中失败（`bundle_dmg.sh`，疑与磁盘空间有关），需在干净环境复现。
- 发布前：公证、签名后冒烟、Windows 验证、`docs/assistant-engines.md` 验收表填写。
