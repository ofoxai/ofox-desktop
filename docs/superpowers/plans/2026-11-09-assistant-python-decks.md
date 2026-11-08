# 助手 P1：用 Python 写 PPT（macOS）Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在 macOS 上让助手用 Python（python-pptx + 辅助库 + matplotlib + SVG 图）做 PPT，先预览再交付，deck 对象保留为兜底。

**Architecture:** 宿主把模型的 Python 代码写进文档目录，像跑 officecli 一样（同一套 `Work` / `run_engine` 与 `sandbox-exec`）用打包的 Node 运行 `python/runner.mjs`；runner 在 Pyodide 里执行代码，产出 `out.pptx` 与 `lint.json`。之后复用 officecli 的校验、溢出修正与每页 SVG。`preview: true` 不建附件，运行时把每页 SVG 拼成总览图给模型；模型改代码再交付，或用 `fromPreview` 交付预览过的文件。

**Tech Stack:** Rust（tokio、serde）、Node 22（node:test、@napi-rs/canvas）、Pyodide 314.0.7、python-pptx 1.0.2、matplotlib 3.10.8、officecli 1.0.160。

**Spec:** `docs/superpowers/specs/2026-11-09-assistant-python-documents-design.md`

**验证代码（起点，git 已忽略，只在本工作区）：** `.superpowers/spike-pptx/deck.py`（辅助库）、`rules_py.md`（规则）、`gen_py.mjs`（Pyodide 的加载与执行方式）。

## Global Constraints

- Python 引擎只在 macOS 提供，且只有 `sandbox-exec` 探测通过才可用；否则视为不可用，走 deck 对象（失败即关闭）。
- Pyodide `314.0.7`。二进制包取 Pyodide 锁文件的版本与 SHA-256：numpy 2.4.6、matplotlib 3.10.8 及依赖（contourpy、cycler、fonttools、kiwisolver、packaging、pyparsing、python-dateutil、pytz、six）、lxml 6.1.3、pillow 12.2.0、typing_extensions 4.15.0。纯 Python 包取 PyPI 并固定 SHA-256：python-pptx 1.0.2、python-docx 1.2.0、openpyxl 3.1.5、XlsxWriter 3.2.9、et_xmlfile 2.0.0。字体：思源黑体 SC 子集 OTF（OFL）。
- 所有资源在打包时下载并校验；应用运行时不下载任何东西。
- Python 代码 ≤ 200 KB；一次运行 90 秒超时；同一回复里 Python 文档串行。
- 同一文件名每次回复最多 2 次预览；预览不计入 `MAX_DOCUMENTS_PER_RUN`。
- 给模型的报错用英文，沿用 `refuse(FAILED, …)` 的写法；界面文案不变。
- 提交信息简洁、无 Generated / Co-Authored-By；只提交不推送；不提交 key。
- 改了 `assistant-runtime/src` 之后、跑宿主 e2e 之前，重新暂存：`env -u APPLE_SIGNING_IDENTITY node scripts/assistant-prepare.mjs`。

## Review Focus

1. 模型预览后没交付就结束回复 → 用户仍拿到文件：回复结束时自动交付每个文件名最新的一次预览（Task 6）。
2. 同时带 `fromPreview` 与 `content`，或给 docx / xlsx 发 python 代码 → 明确拒绝并说明用法（Task 5、6）。
3. 代码跑满 90 秒（大量 matplotlib 图）→ 进程结束，报错写明秒数并建议减少图（Task 4，用可注入的超时测）。
4. 资源目录缺 wheel 或字体 → 引擎报告不可用、工具说明不提 python（Task 1、4）。
5. 用户中途停止回复 → runner 子进程随之结束（Task 9）。

---

### Task 1: 固定并暂存 Python 资源

**Files:**
- Create: `scripts/python-downloads.json`
- Modify: `scripts/assistant-prepare.mjs`
- Modify: `assistant-runtime/package.json`（dependencies 加 `"pyodide": "314.0.7"`）
- Test: `assistant-runtime/test/packaging.test.mjs`

**Interfaces:**
- Produces: `resources/engines/python/wheels/*.whl`、`resources/engines/python/fonts/NotoSansSC-Regular.otf`、`resources/engines/python/assets.json` = `{ "pyodide": "314.0.7", "binary": [包名…], "pure": [wheel 文件名…], "font": "fonts/NotoSansSC-Regular.otf" }`；`manifest.json` 增加 `engines.python = { "path": "engines/python/assets.json", "runner": "runtime/src/python/runner.mjs", "version": "314.0.7" }`；`engines/source.json` 增加 `python` 戳。

- [ ] **Step 1: 写失败测试** `python assets are pinned and staged`：`python-downloads.json` 每项有 `name`、`https://` 的 `url`、64 位十六进制 `sha256`；二进制 wheel 的 SHA 与 `pyodide-lock.json` 同名包一致；Global Constraints 的每个包恰好一项；暂存后 `assets.json` 列出的文件都存在。
- [ ] **Step 2: 运行确认失败** `pnpm assistant:test` → FAIL。
- [ ] **Step 3: 实现**：二进制 wheel 用 `https://cdn.jsdelivr.net/pyodide/v314.0.7/full/<file_name>`；纯 Python wheel 用 PyPI 文件 URL；字体用 notofonts/noto-cjk 某个发布标签下的 `Sans/SubsetOTF/SC/NotoSansSC-Regular.otf`（下载一次记下 SHA）。`assistant-prepare.mjs` 照 officecli / typst 的方式下载、校验、暂存、写戳，戳不变则跳过。
- [ ] **Step 4: 运行确认通过** `node scripts/assistant-prepare.mjs && pnpm assistant:test` → PASS。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): pin and stage the Python engine assets"`

### Task 2: runner

**Files:**
- Create: `assistant-runtime/src/python/runner.mjs`
- Create: `assistant-runtime/src/python/deck.py`（先原样复制 `.superpowers/spike-pptx/deck.py`；Task 3 再改）
- Test: `assistant-runtime/test/python-runner.test.mjs`

**Interfaces:**
- Consumes: Task 1 的 `assets.json` 与资源目录。
- Produces: `node runner.mjs --assets <dir> --code <code.py> --out <out.pptx> --lint <lint.json>`。成功退出码 0，写 `out.pptx` 与 `lint.json`（字符串数组）。失败退出码 1，stderr 为 `PythonError: <回溯末尾 ≤ 1800 字符>` 或 `RunnerError: <原因>`。Python 里可 `import host`，`host.svg_to_png(svg, w, h) -> base64 PNG`。

- [ ] **Step 1: 写失败测试**（用暂存的 Node 跑 runner；资源缺失时测试失败并提示 `run pnpm assistant:prepare first`，不跳过）：
  - `makes a pptx from code`：`prs = new_deck(title="t")` 加一页 `content_slide` → 退出 0；`out.pptx` 含 `ppt/slides/slide1.xml`；`lint.json` 是数组。
  - `reports a Python error`：引用未定义名 → 退出 1，stderr 含 `NameError`。
  - `requires prs`：不建 `prs` → 退出 1，stderr 含 `the code must create prs = new_deck(...)`。
  - `writes non-negative chart axis ids`：加一个 `XL_CHART_TYPE.COLUMN_CLUSTERED` 图表 → 每个 `ppt/charts/chart*.xml` 里 `c:axId` 与 `c:crossAx` 的 `val` 都是非负整数，且两两对应一致。
  - `fails on a missing wheel`：用少一个 wheel 的资源目录副本 → 退出 1，stderr 含 `RunnerError` 与该文件名。
  - `draws an SVG figure`：调用 `figure(slide, "<svg …>", …)` → `out.pptx` 含一张 PNG。
- [ ] **Step 2: 运行确认失败** `pnpm assistant:test` → FAIL。
- [ ] **Step 3: 实现 `runner.mjs`**：按 `gen_py.mjs` 的方式加载 Pyodide，包只从资源目录加载；纯 Python wheel 用 `pyodide.unpackArchive(bytes, "wheel")` 装入；字体写到 `/fonts`；`deck.py` 写进 site-packages；注册 `host` 模块；执行代码后把图表部件中的负轴编号映射为正数，再保存。
- [ ] **Step 4: 运行确认通过** `pnpm assistant:test` → PASS。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): run deck code in Pyodide from a runner script"`

### Task 3: 模板与辅助库

**Files:**
- Create: `scripts/assistant-deck-template.mjs`（生成模板；pptxgenjs 只作 devDependency）
- Create: `assistant-runtime/src/python/template.pptx`（脚本产物，提交）
- Modify: `assistant-runtime/src/python/deck.py`、`assistant-runtime/src/python/runner.mjs`
- Test: `assistant-runtime/test/python-deck.test.mjs`

**Interfaces:**
- Produces（模型可见，之后不得改名）：`new_deck(title="", footer="")`、`cover_slide(prs, title, subtitle="", kicker="", meta="", notes="", art=None)`、`section_slide(prs, title, number="", subtitle="", notes="")`、`content_slide(prs, title, subtitle="", notes="")`、`card(slide, x, y, w, h, title, body="", points=None, highlight=False, size=13)`、`kpi(slide, x, y, w, h, value, label, note="", highlight=False)`、`steps(slide, items, x, y, w, h, highlight=-1, size=12)`、`timeline(slide, items, x, y, w, h, highlight=-1, size=12)`、`bullets(slide, items, x, y, w, h, size=13)`、`callout(slide, y, message, x=None, w=None, h=0.62)`、`formula(slide, x, y, w, h, lines, size=15)`、`text`、`shape`、`arrow`、`figure(slide, svg, x, y, w, h, alt="figure")`、`plot(slide, fig, x, y, w, h, alt="chart")`；`T`、`G`、`LINT`。
- 模板版式名 `Cover`、`Section`、`Content`；`Content` 有标题占位符、页脚与页码，不带日期占位符。

- [ ] **Step 1: 写失败测试**（经 runner 执行，用 officecli 检查）：
  - `uses the 16:9 template`：幻灯片 13.333 × 7.5 英寸；内容页的版式名为 `Content`；`view outline` 每页标题非空。
  - `renders no stray placeholders`：每页 XML 没有 `type="dt"` 占位符；第 1 页 SVG 不出现日期文本。
  - `validates clean`：封面 + 章节 + 两页内容（card、kpi、steps、timeline、callout、formula、plot、原生图表）→ `validate` 零错误、`view issues` 零问题。
  - `lints what will not fit`：`card` 高 0.8 英寸放三条要点 → `LINT` 含 `text needs`；`steps` 框高为所需三倍 → 含 `lower h`；`figure` 的 SVG 字号换算后小于 10pt → 含 `smallest text`；`plot` 中刻度 8pt → 含 `use at least 10`。
  - `draws Chinese in plots`：matplotlib 标题含中文时不出现缺字警告（捕获 `UserWarning: Glyph … missing`）。
- [ ] **Step 2: 运行确认失败** → FAIL。
- [ ] **Step 3: 实现**：模板脚本输出 16:9、三个版式的 pptx 并确保 `validate` 零错误；`deck.py` 改为打开 `/template.pptx` 并按版式名取版式；单行文字框高度按 1.4 倍行高；保留验证版的写时检查。
- [ ] **Step 4: 运行确认通过** → PASS。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): add the 16:9 deck template and Python helpers"`

### Task 4: 宿主发现 Python 引擎并在沙箱里运行它

**Files:**
- Modify: `src-tauri/src/services/assistant/documents/engines.rs`
- Modify: `src-tauri/src/services/assistant/documents/create.rs`（`Work` 加字段；新沙箱配置；探测）
- Modify: `src-tauri/src/services/assistant/host.rs`（`python_sandbox: EngineSandbox`；bootstrap 的 `documents.python`）

**Interfaces:**
- Produces: `pub struct PythonEngine { pub node: PathBuf, pub runner: PathBuf, pub assets: PathBuf }`；`Engines.python: Option<PythonEngine>`（只在 macOS 构建、且三个路径都在场时为 `Some`）；`Work` 增加 `read_root: Option<&Path>`（沙箱可读的引擎根目录，缺省为 exe 所在目录）与 `timeout: Duration`（officecli / typst 仍为 `ENGINE_TIMEOUT`）；常量 `PYTHON_TIMEOUT = 90s`、`PYTHON_SANDBOX`（照 `OFFICE_SANDBOX` 写，可读根为运行时资源目录）；`AssistantHost::python_engine(&self, work) -> Option<…>`：探测通过才返回；bootstrap 的 `runtime.documents.python: bool`。

- [ ] **Step 1: 写失败测试**（`engines.rs` 与 `create.rs` 的 `#[cfg(test)]`，沿用已有的 stand-in `sandbox-exec` 测试模式）：
  - `engines_at_reads_the_python_engine`：manifest 有 `engines.python` 且三个文件在场 → `Some`，路径都在 base 内；缺 `assets.json` → `None`。
  - `python_runs_under_sandbox_exec_with_its_profile`：stand-in `sandbox-exec` 记录的参数里有 `-p` 与 Python 配置、`ENGINE=` 指向资源根。
  - `python_is_unavailable_when_the_probe_fails`：stand-in 探测失败 → `python_engine` 返回 `None`，并且不会不带沙箱运行。
  - `a_slow_engine_is_stopped_at_its_timeout`：`Work.timeout` 设 1 秒、exe 为 sleep 脚本 → 报错含 `did not finish within 1 seconds`。
  - `bootstrap_reports_python_availability`：无 Python 引擎时 `documents.python == false`。
- [ ] **Step 2: 运行确认失败** `cargo test --manifest-path src-tauri/Cargo.toml documents` → FAIL。
- [ ] **Step 3: 实现**上述接口；非 macOS 编译时 `python` 恒为 `None`。
- [ ] **Step 4: 运行确认通过**，并跑 `cargo fmt --check` 与 `cargo clippy -- -D warnings`。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): find the Python engine and run it in its own sandbox"`

### Task 5: 用 Python 代码生成 pptx

**Files:**
- Modify: `src-tauri/src/services/assistant/documents/create.rs`

**Interfaces:**
- Consumes: Task 4 的 `python_engine`、Task 2 的 runner 命令行。
- Produces: `CreateDocumentArgs { name, format, content: Option<Value>, preview: bool (#[serde(default)]), from_preview: Option<String> (#[serde(rename = "fromPreview")]) }`；`Content::Python(String)`；`async fn make_python(python: &Work, office: &Work) -> Result<Made, String>`；从 `make_office` 拆出的 `async fn check_office(work: &Work, format, issues: Vec<Issue>) -> Result<Made, String>`（validate → view issues → `fix_layout` → stats → `svg_pages`），`make_office` 与 `make_python` 共用。

- [ ] **Step 1: 写失败测试**（假 runner：一个把夹具 pptx 复制成 `out.pptx` 并写 `lint.json` 的脚本；假 officecli 沿用现有夹具）：
  - `python_content_makes_a_pptx_and_reports_lint`：结果 `issues` 含 lint 文本（warning），文件存在，`svgPages` 与页数一致。
  - `a_python_error_reaches_the_model`：runner stderr `PythonError: NameError: name 'x' is not defined` → 报错含这句。
  - `python_content_is_pptx_only`：docx 带 python 内容 → 拒绝，含 `python content is for pptx only`。
  - `python_content_needs_the_engine`：无 Python 引擎 → 拒绝，含 `send a deck object instead`。
  - `oversized_python_code_is_refused`：代码 200 KB + 1 字节 → `TOO_LARGE`。
  - 现有 deck 与批处理测试全部继续通过。
- [ ] **Step 2: 运行确认失败** → FAIL。
- [ ] **Step 3: 实现**：`Document::new` 识别 `{"kind":"python","code":…}`（也接受同形 JSON 字符串）；写 `code.py`；运行 runner（`timeout: PYTHON_TIMEOUT`，同一回复串行）；读取 `lint.json` 转成 warning；再 `check_office`。
- [ ] **Step 4: 运行确认通过**，加 fmt / clippy。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): make a pptx from Python deck code"`

### Task 6: 预览与交付

**Files:**
- Modify: `src-tauri/src/services/assistant/documents/create.rs`
- Modify: `src-tauri/src/services/assistant/media_tools/host.rs`（`MediaRun` 记录预览）

**Interfaces:**
- Produces: `MediaRun::record_preview(name: &str, doc_id: &str) -> Result<(), String>`（同名超过 2 次返回错误）、`MediaRun::take_preview(doc_id: &str) -> Option<PreviewRecord>`、`MediaRun::undelivered_previews() -> Vec<PreviewRecord>`；预览结果 JSON：`{ "status": "completed", "preview": true, "previewId": doc_id, "slides": n, "issues": [...], "pagePaths": [...] }`（`pagePaths` 只给运行时，运行时不转给模型）；交付沿用现有附件结果。常量 `MAX_PREVIEWS_PER_NAME = 2`。

- [ ] **Step 1: 写失败测试**：
  - `a_preview_makes_no_attachment`：`preview: true` → 无附件、无 `publish_generation`，返回 `previewId` 与全部页的路径。
  - `from_preview_delivers_the_previewed_file`：再以 `fromPreview` 调用 → 一份附件，字节与预览文件相同，不再运行引擎。
  - `from_preview_rejects_unknown_ids_and_extra_content`：别的回复或不存在的 id → 拒绝；同时带 `content` → 拒绝，含 `send either content or fromPreview`。
  - `a_third_preview_is_refused`：同名第 3 次预览 → 拒绝，含 `at most 2 previews`。
  - `an_undelivered_preview_is_delivered_at_the_end`：只预览、未交付即结束回复 → 自动交付该名最新一次预览，一份附件。
  - `previews_do_not_count_toward_the_document_limit`。
- [ ] **Step 2: 运行确认失败** → FAIL。
- [ ] **Step 3: 实现**；回复结束清理文件夹之前先自动交付未交付的预览。
- [ ] **Step 4: 运行确认通过**，加 fmt / clippy。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): preview a deck before delivering it"`

### Task 7: 运行时的 create_document

**Files:**
- Modify: `assistant-runtime/src/document-tools.mjs`
- Modify: `assistant-runtime/src/thumbnails.mjs`
- Test: `assistant-runtime/test/document-tools.test.mjs`、`assistant-runtime/test/thumbnails.test.mjs`

**Interfaces:**
- Consumes: bootstrap 的 `documents.python`；Task 6 的预览结果。
- Produces: 工具参数增加 `content` 的 `{ kind: "python", code: string ≤ 200000 }` 形态、`preview?: boolean`、`fromPreview?: string`（`content` 在带 `fromPreview` 时可省略）；`export async function contactSheet(svgBuffers: Buffer[], { columns = 2, width = 1600 } = {}) -> { data, mimeType }`（复用 `slideText` 绘字）。

- [ ] **Step 1: 写失败测试**：
  - `mentions python only when the host offers it`：`documents.python` 为 true 时说明与 guidelines 含 `kind":"python"` 与 `preview`；为 false 时不含。
  - `passes python content to the host unchanged`：不经 deck 渲染。
  - `a preview result carries one contact sheet`：能看图时结果有一张图，文本含 `previewId`、页数、检查结果与 “deliver with fromPreview”；不能看图时只有文本；`pagePaths` 不出现在给模型的内容里。
  - `contact sheet draws every slide`：3 张 SVG → 图宽 1600、两列两行，每格内有非白像素（文字区域亮像素检查沿用 `lightPixels` 写法）。
- [ ] **Step 2: 运行确认失败** → FAIL。
- [ ] **Step 3: 实现**。
- [ ] **Step 4: 运行确认通过** `pnpm assistant:test` → PASS。
- [ ] **Step 5: 提交** `git commit -m "feat(assistant): preview Python decks in create_document"`

### Task 8: 技能改写

**Files:**
- Modify: `assistant-runtime/skills/ofox-office-pptx/SKILL.md`
- Test: `assistant-runtime/test/bundled-skills.test.mjs`

**Interfaces:**
- Consumes: Task 3 的辅助库接口；Task 6 / 7 的预览流程。

- [ ] **Step 1: 写失败测试**：
  - `the pptx skill names only helpers that exist`：从 SKILL.md 的辅助库一节取出全部函数名，每个都在 `deck.py` 中有 `def`。
  - `the pptx skill describes preview then deliver`：含 `preview: true`、`fromPreview` 与 “Python is unavailable” 时用 deck 对象的一节。
- [ ] **Step 2: 运行确认失败** → FAIL。
- [ ] **Step 3: 改写**：以 `rules_py.md` 为底——先写逐页计划；设计规则；辅助库接口；SVG 与 matplotlib 要求；先预览再交付（最多 2 次）；原 deck 对象内容保留为兜底一节。
- [ ] **Step 4: 运行确认通过** → PASS。
- [ ] **Step 5: 提交** `git commit -m "docs(assistant): teach the pptx skill the Python route"`

### Task 9: 宿主 e2e（真引擎）

**Files:**
- Modify: `src-tauri/tests/assistant_host.rs`

- [ ] **Step 1: 写测试**（沿用 `create_document(bundle, model, skill, args)` 帮手与真实暂存的运行时）：
  - `previews_then_delivers_a_python_deck`：假模型先发 python 代码 + `preview: true`，再发 `fromPreview` → 一份附件，`validate` 零错误，issues 为零，`pages/1..N.svg` 存在。
  - `falls_back_to_the_deck_object_without_python`：去掉 manifest 的 `engines.python` → 说明不提 python，deck 对象照常生成。
  - `stopping_the_reply_ends_the_runner`：runner 运行中取消 → 回复结束后没有残留的 runner 进程（按进程命令行查找）。
- [ ] **Step 2: 重新暂存并运行** `env -u APPLE_SIGNING_IDENTITY node scripts/assistant-prepare.mjs && cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host document` → PASS。
- [ ] **Step 3: 提交** `git commit -m "test(assistant): prove the Python deck route end to end"`

### Task 10: 回归脚本与文档

**Files:**
- Modify: `src-tauri/tests/assistant_host.rs`（`real_eval` 模块的断言）
- Modify: `scripts/assistant-real-eval.mjs`（报告增加列）
- Modify: `docs/assistant-engines.md`、`docs/assistant-roadmap.md`、spec 的状态行

- [ ] **Step 1: 写失败测试** `node scripts/assistant-real-eval.mjs --self-test`：报告表头含 `route`（python / deck）与 `previews` 两列。
- [ ] **Step 2: 实现**：pptx 用例记录走的路线与预览次数，并断言最终只有一份附件、零 warning；文档写明 Python 引擎、隔离方式、预览流程与兜底；路线图加 P1 与后续切片（Word / Excel 用 Python、Windows 沙箱）。
- [ ] **Step 3: 运行** `node scripts/assistant-real-eval.mjs --self-test && pnpm assistant:test && pnpm assistant:smoke` → PASS。
- [ ] **Step 4: 提交** `git commit -m "docs(assistant): document the Python deck route and its eval"`
- [ ] **Step 5: 用户验收**：用户用钥匙串里的 key 跑 `pnpm assistant:real-eval`，并在真机上做“讲解 Transformer”，与对照样例并排看。
