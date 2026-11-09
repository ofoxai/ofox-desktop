# 助手 P1：用 Python 写 PPT（macOS 先行）— 设计稿

日期：2026-11-09
状态：已实现（2026-11-09 代码、假模型 e2e 与回归脚本完成），待用户验收：`pnpm assistant:real-eval` 与真机“讲解 Transformer”对照。

前置：M3 设计稿 `2026-11-07-assistant-office-polish-and-edit-design.md`（M3a-1 已交付 deck 对象路线）。
调研：`docs/superpowers/research/2026-11-08-ppt-quality-gap.md`、`2026-11-08-python-runtime.md`。

## 为什么做

M3a-1 的 deck 对象能稳定出文件，但真机验收认为“质量太差”：每页一排小卡片、平均 90 字、没有图、版式单调。
对照样例（强模型自由写 python-pptx、用 matplotlib 画图）每页约 334 字、2–4 个信息区、有讲机制的图。
两轮验证证明：让模型写代码、配一层辅助库和写时检查、再看一轮渲染图自查，terra / Luna / DeepSeek Flash 都能做出达到或超过
对照样例的 PPT；python-pptx 与 pptxgenjs 质量相当，Python 代码更短、画数据图更顺手，并且后续 Word、表格、改用户文件都要用到。

用户的决定（2026-11-09）：统一用 Python（Pyodide）；macOS 先行，Windows 继续用 deck 对象，等 Windows 沙箱做完再开。

## 目标

1. macOS 上，助手做 PPT 时写 Python：python-pptx + 我们的辅助库 + matplotlib + SVG 示意图，在隔离的子进程里运行，产出原生可编辑的 pptx。
2. 先预览再交付：第一次调用只出预览（整套缩略图 + 检查结果），模型据此改一版再交付；用户只看到一张卡片。
3. 宿主保底：校验、修正已知的 XML 问题、自动修正溢出、生成每页 SVG，与 M3a-1 一致。
4. 隔离可证：模型的代码不能联网、不能写文档目录以外的地方、不能读用户目录里的其他文件，有测试证明。
5. deck 对象保留为兜底：Windows / Linux、Python 引擎不可用时，以及模型自己选用时。

## 不做的（这一轮）

- Windows / Linux 上的 Python（等系统沙箱，单独切片）。
- Word、Excel 用 Python 生成或修改（下一个切片，复用本轮的运行时）。
- 用户发起的修改与卡片替换（`revises`，M3a-2）、主题切换、图标库、AI 插图。
- 运行时联网装包（`micropip`）、调用子进程（ffmpeg 等）。

## 方案总览

```
模型 ──create_document({format:"pptx", content:{kind:"python", code}, preview:true})──► 运行时(Node)
运行时 ──host tool──► 宿主(Rust) prepare 文档目录，写 code.py
宿主 ──spawn (sandbox-exec PYTHON_SANDBOX, 干净环境, 失效代理, 90s)──► node python-runner.mjs
runner：Pyodide + 本地 wheel + deck.py + 模板 → out.pptx、lint.json（只写文档目录）
宿主：officecli validate → 修 python-pptx 已知问题 → view issues → 溢出修正(≤2 轮) → 每页 SVG
运行时：每页 SVG 拼成总览图（已有的带字缩略图）+ 检查结果 → 回给模型（preview 时不建附件、不出卡片）
模型 ──create_document({... content 改过的代码 }) 或 ({... fromPreview:"<id>"})──► 交付：建附件、出一张卡片
```

## 组件

### 1. Python 引擎（运行时资源）

- Pyodide 314.0.7（CPython 3.14）核心约 13 MB；本地 wheel：numpy、matplotlib（及依赖）、lxml、Pillow 约 19 MB；
  纯 Python 的 python-pptx、python-docx、openpyxl、XlsxWriter 等约 9 MB；思源黑体子集（OFL，matplotlib 中文）；我们的 16:9 模板；`deck.py`。
- 由 `scripts/assistant-prepare.mjs` 按锁定的版本和 SHA-256 下载、放进 `resources/engines/python/`，记进引擎清单；
  `engines.rs` 增加 `python` 引擎（runner 与资源目录都在场才算可用）。不在运行时下载任何东西。
- 入口 `python-runner.mjs`（随运行时发布）：读文档目录里的 `code.py`，加载本地包（禁止网络加载）、写入 `deck.py` 与模板、
  执行代码、保存 `out.pptx`，把辅助库的检查写入 `lint.json`，错误时把 Python 回溯（截短）写到 stderr 并以非零退出。
  隔离靠独立进程和系统沙箱；收窄 Pyodide 暴露给 Python 的 JS 全局对象经验证无效，不做。

### 2. 隔离（宿主）

- Pyodide 本身不是安全边界（调研已证实 Python 能接触宿主 JS 环境），所以安全由进程 + 系统沙箱保证。
- 宿主像跑 officecli 一样跑 runner：`run_engine` 的干净环境、私有 HOME、失效代理、输出上限；超时 90 秒（matplotlib 首次建字体缓存较慢）。
- 新的 `PYTHON_SANDBOX` 配置：禁止网络；禁止写入，除了本次文档目录；禁止读取 `/Users`、`/Volumes` 下的内容，除了本次文档目录和引擎目录
  （runner、Node、Pyodide 资源、运行时里 runner 需要的模块）。
- **失败即关闭**：`sandbox-exec` 不存在或探测不通过时，Python 引擎视为不可用，create_document 退回 deck 对象路线；
  不像 officecli 那样“没有沙箱也照跑”。非 macOS 平台不提供 Python 引擎。
- 同一回复里的 Python 文档串行执行，避免多个 600 MB 进程同时存在；进程结束即释放内存。

### 3. 辅助库与模板

- `deck.py`：验证版的 Python 移植，接口不变——`new_deck`、`cover_slide`、`section_slide`、`content_slide`（真标题占位符 + 论点副标题）、
  `card`、`kpi`、`steps`、`timeline`、`bullets`、`callout`、`formula`、`text`、`shape`、`arrow`、`figure`（SVG）、`plot`（matplotlib）。
  写时检查：文字所需高度（单行 1.4 倍行高，与 officecli 检查器一致）、框高超过所需两倍、图内最小字号低于 10pt、SVG 比例不符。
- 模板：我们自己的 16:9 pptx（封面 / 章节 / 内容三个版式，内容版式有标题占位符、页脚与页码），由脚本生成并提交，
  解决 python-pptx 默认模板留下的日期、页脚占位符和 4:3 版式。
- 主题沿用 navy 的角色色（主色、深色、强调色、三档灰、两种浅底）；字体名 Microsoft YaHei 并写东亚字体。

### 4. create_document 的变化

- pptx 的 `content` 多一种：`{"kind":"python","code":"…"}`（≤ 200 KB）。原有 deck 对象和批处理照旧。
- `preview: true`：宿主照常做文件、跑检查和每页 SVG，但不建附件、不发卡片，返回 `previewId`、页数、检查结果；
  运行时把每页 SVG 拼成一张总览图（两列，带字）随结果给能看图的模型。
- 交付：同一文档再调一次，不带 `preview`（代码可改），或带 `fromPreview: "<previewId>"` 直接交付预览过的文件、不必重发代码。
  每份文档最多 2 次预览；预览文件随回复结束清理。
- Python 引擎不可用时，content 为 python 的调用被拒绝并说明“用 deck 对象”；工具说明只在引擎可用时提 python。

### 5. 宿主后处理

`validate` → 修正 python-pptx 的已知问题（图表坐标轴编号为负数时改成正数）→ `view issues` → 现有溢出修正（≤ 2 轮）→ 每页 SVG。
辅助库的 `lint.json` 与 officecli 的问题一起作为 warning 回给模型。Python 报错（语法、未定义名、SVG 解析失败）原样截短返回，模型改代码重试。

### 6. 技能

`ofox-office-pptx` 改写：macOS 上首选 Python 路线——先写逐页计划（论点 / 视觉 / 版式），设计规则（每页 2–4 个信息区、每页一个讲解型视觉、
相邻页版式不同、8 页以上至少一页章节页和一页数字页、强调色只给重点、最小 10pt、按估算留够空间）、辅助库接口、SVG 与 matplotlib 的要求、
先预览再交付的流程。deck 对象一节保留，标明“Python 不可用时用”。

## 最容易出问题的地方（每条都要有测试）

1. **隔离**：测试脚本尝试联网、写文档目录以外、读用户目录里的其他文件，三者都必须失败；探测失败时 Python 引擎不可用、deck 对象路线照常。
2. **资源与版本**：引擎清单缺文件、SHA 不符、Pyodide 加载失败 → 引擎不可用而不是半可用；不触网。
3. **超时与内存**：死循环的代码在 90 秒被终止并返回明确错误；同回复多文档串行。
4. **预览与交付**：预览不建附件、不出卡片；`fromPreview` 只能交付本回复、本文档的预览；第三次预览被拒；回复结束清理预览目录。
5. **模型的错误**：没有代码块、未定义名、SVG 解析失败、没有创建 `prs`，都返回可操作的报错而不是“文档失败”。
6. **兜底**：Windows、Linux、无沙箱的 macOS 上，create_document 的说明和行为都只走 deck 对象，M3a-1 的测试全部继续通过。
7. **输出合法**：模板 + 辅助库产出的文件 `validate` 零错误；修正坐标轴编号后零错误。

## 验收

- 宿主 e2e（真引擎，假模型）：python 代码 → 预览（无附件）→ 交付（一份附件、`validate` 通过、issues 为零、`pages/1..N.svg`）；隔离三项失败；超时终止。
- 真实模型：`pnpm assistant:real-eval` 增加 python 模式，四个生成提示 × terra / Luna / DeepSeek Flash：每次只有一份附件、零 warning、无字面 `\n`；
  记录耗时与 token。
- 用户真机：同题“讲解 Transformer”，与对照样例并排看，用户认可。

## 已决定

1. PPT 统一走 Python（python-pptx），deck 对象保留为兜底（2026-11-09 用户定）。
2. 运行时用 Pyodide，包随应用发布，运行时不下载（调研：官方 CDN 在国内 15 KB/s）。
3. 安全靠子进程 + `sandbox-exec`，失败即关闭；收窄 Pyodide 的 JS 全局经验证无效，不做（2026-11-09 调研更正）。
4. macOS 先行；Windows 等系统沙箱（2026-11-09 用户定）。
5. 自查用“预览 + 交付”两步完成，不在本轮做 `revises` 卡片替换。
6. 主题仍是 navy 一套；多主题、图标、AI 插图留到后续。

## 留给实现者决定

- 思源黑体子集的字符范围（建议 GB2312 + 常用标点 + 拉丁），只要中文图表不出方块。
- 总览图的分辨率与每行页数，只要 8 页能在一张图里看清标题和正文。
