# 助手产出文件 M1–M2（引擎打包 + create_document）实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 助手能把 PDF、DOCX、PPTX、XLSX（以及 md/txt/csv/html/json）作为真实文件交付：officecli 和 Typst 随包分发，运行时新增 `create_document` 工具，宿主生成并校验文件，对话里出现文档卡片，能保存并打开；四个随包办公技能教模型怎么写内容。

**Architecture:**
- 引擎二进制由 `scripts/assistant-prepare.mjs` 按 `engine-downloads.json` 下载、SHA-256 校验、放进 `assistant-runtime/resources/engines/`，和 Node 一起签名，路径写进 `resources/manifest.json`。
- 模型只写内容：Office 三种格式写 officecli `batch` 命令数组，PDF 写 Typst 正文，文本格式写字符串。运行时工具 `create_document` 经 `hostTool` 交给 Rust；Rust 在会话私有目录 `<sid>/outputs/<docId>/` 里 spawn 引擎、校验、导入为 `origin:"generated"` 的附件、发 `generation` 事件；运行时拿到路径后出缩略图回传给视觉模型。
- 随包技能（`assistant-runtime/skills/ofox-office-*`）由宿主并入 1.5.1 的技能清单（单独的 bundled 表），`use_skill` 直接读随包目录。

**Tech Stack:** Node 22 + Pi SDK 1.5.0（运行时，node:test）；Rust / Tauri 2（宿主，inline `#[cfg(test)]` + `src-tauri/tests/assistant_host.rs` e2e）；React 18 + TS（Vitest + RTL）；officecli v1.0.160（Apache-2.0，C#/.NET 自包含）；Typst v0.15.1（Apache-2.0，Rust）。

**Spec:** `docs/superpowers/specs/2026-11-07-assistant-deliverables-and-media-design.md`（第一部分 A1–A6、里程碑 M1/M2）。M3（inspect/edit_document、PDF 内嵌图片）、M4（媒体）、M5（收尾）另写计划。

## Global Constraints

- **工作区**：只在 `/Users/yunshen/code/ofox-desktop-1.5.2` 工作，分支 `feat/assistant-deliverables`。不 checkout、不修改、不推送 `release/1.5.0`、`feat/assistant-1.5.1`、`feat/assistant-media-agent`。不打 tag。每个任务完成后本地提交；不推送、不开 PR。
- **1.5.1 是冻结基线**：它若有新提交，`git merge` 进来，不 rebase。技能清单的改动（Task 11）单独提交。
- **磁盘**：开工前、每次 Rust 全量编译或 `pnpm tauri build` 前先 `df -h /System/Volumes/Data`；低于 20 GB 时只能删 `/Users/yunshen/code/ofox-desktop/src-tauri/target`、`/Users/yunshen/code/ofox-desktop-assistant-media/src-tauri/target`、本工作区 `src-tauri/target/debug/incremental`。同一时间只跑一个重任务（`cargo build/test/clippy`、`pnpm test:unit`、`pnpm assistant:test`、`pnpm tauri build` 不并行）。不要在 `/Applications` 安装或启动新版 app。
- **开源边界**：officecli、Typst 的 LICENSE/NOTICE 进 `assistant-runtime/notices/`；OfficeCLI 技能改编要注明来源（Apache-2.0）；不拿 Anthropic 的 pdf/docx/pptx/xlsx 技能；提交信息和注释不提闭源产品。
- **sidecar 限制**：运行时只有自带 Node 22，没有 npm/Python；生产依赖 `npm ci --omit=dev --ignore-scripts`。运行时→宿主每帧 ≤ 8 MiB（`host.rs:67 FRAME_LIMIT`），宿主→运行时每行 ≤ 1 MiB（`ipc.mjs:4 LINE_LIMIT`，宿主侧 `host.rs:2260`）。工具调用上限：本机配置默认开启时 60 步 / 900 s / $1（`capabilities.rs:39-56`），无本机工具时 20（`runtime.mjs:42`）。
- **附件上限**：单文件 25 MiB（`attachments.rs:26 MAX_FILE_BYTES`），会话 500 MiB。
- **签名**：macOS 上每个 Mach-O 都签（hardened runtime + 时间戳）；officecli 需要 JIT 类 entitlements（Task 2）。发布平台 macOS arm64 和 Windows x64。
- **界面文字三语**（`src/i18n/locales/{zh,en,ja}.json`），不写死在代码里。
- **Ofox 地址**用 `src/lib/ofoxUrls.ts` + `useOfoxApex`。
- **Windows/macOS 行为一致**，路径用平台 API 拼接；子进程环境经 `process::clean_environment`。
- **项目规范**：pnpm；Conventional Commits，不带生成信息；每个提交可编译、测试全过；Rust 过 `cargo fmt --check --manifest-path src-tauri/Cargo.toml` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`；运行时 `src` 改动后先 `pnpm assistant:prepare` 再 `pnpm assistant:test`；任何 Cargo 构建/测试前先 `pnpm assistant:prepare`（AGENTS.md:17）。
- **密钥**：新工具的参数、结果、日志沿用 `scrubSecrets` / `redact`；引擎子进程不继承 app 凭据。

## Review Focus

1. **模型把内容写在聊天里而不调工具**：假模型测不出"不调"，所以用测试钉住系统提示的措辞——Task 8 的 `runtime.test.mjs` 断言系统提示含 "produce the file with create_document" 和 "never say you cannot attach"。
2. **batch 内容坏掉**（非法 JSON、未知命令、错误路径）：Task 6 用桩 officecli 返回失败 JSON，断言结果 `status:"failed"` 带逐项 `items`、产出目录里没有附件；Task 12 用真实 officecli 跑一次非法 batch。
3. **Typst 编译失败**（语法错误、缺字体）：Task 6 桩 typst 退出码 1 + stderr，断言错误原样回传；Task 12 真实 typst 跑一次缺右括号的正文。
4. **引擎缺失**：Task 3 断言 manifest 没有 `engines` 时 `Engines` 全空；Task 9 断言此时 `params.documents` 只含文本格式；Task 8 断言没有 `params.documents` 时不注册工具。
5. **缩略图撑爆帧**：Task 8 断言 2000×2000 的预览被缩到 ≤ 200 KB，工具结果序列化后 < 1 MiB。
6. **"保存并打开"打开的是用户保存的那份**：Task 7 断言 `open_path` 收到的路径等于保存对话框返回的路径。
7. **引擎偷偷联网或写用户目录**：Task 5 断言子进程环境有 `OFFICECLI_SKIP_UPDATE=1`、`OFFICECLI_NO_AUTO_RESIDENT=1`，`HOME`/`TMPDIR` 在产出目录内。
8. **officecli 在 hardened runtime 下跑不起来**：Task 2 的本地签名 spike 是门槛，失败走 spec "已决定" 第 2 条。

---

### Task 0: 准备

- [ ] **Step 1: 确认分支、磁盘、依赖**

Run: `cd /Users/yunshen/code/ofox-desktop-1.5.2 && git branch --show-current && git log --oneline -2 && df -h /System/Volumes/Data | tail -1`
Expected: 分支 `feat/assistant-deliverables`，最新提交是 `docs: design for assistant deliverables…`，可用空间 > 20 GB。

Run: `pnpm install --frozen-lockfile && pnpm assistant:install && pnpm assistant:prepare`
Expected: 无报错，`assistant-runtime/resources/manifest.json` 存在。

- [ ] **Step 2: 基线测试**

Run: `pnpm assistant:test 2>&1 | tail -6`
Expected: 全部通过。

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host 2>&1 | grep "test result"`
Expected: `test result: ok`（本地没有 runtime 时这个文件会跳过，看到 skipped 也算正常）。

---

### Task 1: 引擎下载、校验、打包、notices、smoke

**Files:**
- Create: `assistant-runtime/engine-downloads.json`
- Create: `assistant-runtime/notices/officecli.txt`、`assistant-runtime/notices/typst.txt`
- Modify: `scripts/assistant-prepare.mjs`（`cachedDownload` :46 泛化 URL；新增 `prepareEngines()`；主流程 :338 之后、:339 `signMacFiles` 之前调用；manifest :359-375 加 `engines`）
- Modify: `scripts/assistant-smoke.mjs`（:364-373 的 notices 列表加两项；`productionSmoke` 里实际运行两个引擎）
- Modify: `assistant-runtime/.gitignore`（确认 `resources/`、`.cache/` 已忽略，不用改）
- Test: `assistant-runtime/test/packaging.test.mjs`

**Interfaces:**
- Produces: `assistant-runtime/engine-downloads.json`：

```json
{
  "officecli": {
    "version": "1.0.160",
    "darwin-arm64": { "url": "https://github.com/iOfficeAI/OfficeCLI/releases/download/v1.0.160/officecli-mac-arm64", "sha256": "<从发布包 SHA256SUMS 复制>", "executable": "officecli" },
    "darwin-x64":   { "url": ".../officecli-mac-x64", "sha256": "…", "executable": "officecli" },
    "linux-x64":    { "url": ".../officecli-linux-x64", "sha256": "…", "executable": "officecli" },
    "win32-x64":    { "url": ".../officecli-win-x64.exe", "sha256": "…", "executable": "officecli.exe" }
  },
  "typst": {
    "version": "0.15.1",
    "darwin-arm64": { "url": "https://github.com/typst/typst/releases/download/v0.15.1/typst-aarch64-apple-darwin.tar.xz", "sha256": "<下载后计算并固定>", "archive": "typst-aarch64-apple-darwin.tar.xz", "executable": "typst-aarch64-apple-darwin/typst" },
    "darwin-x64":   { "...": "typst-x86_64-apple-darwin.tar.xz" },
    "linux-x64":    { "...": "typst-x86_64-unknown-linux-musl.tar.xz" },
    "win32-x64":    { "url": ".../typst-x86_64-pc-windows-msvc.zip", "sha256": "…", "archive": "typst-x86_64-pc-windows-msvc.zip", "executable": "typst-x86_64-pc-windows-msvc/typst.exe" }
  }
}
```

  有 `archive` 的条目解压后取 `executable`；没有的条目下载即可执行文件。officecli 的 sha256 从 `https://github.com/iOfficeAI/OfficeCLI/releases/download/v1.0.160/SHA256SUMS` 复制；Typst 发布包没有校验文件，首次下载后计算并固定（在 json 顶部 `"_note"` 里写明来源）。
- Produces: `resources/engines/officecli/<executable>`、`resources/engines/typst/<executable>`（chmod 755）、`resources/engines/source.json`（stamp：`{officecli:{version,platform,sha256}, typst:{…}}`）。
- Produces: `resources/manifest.json` 新字段 `engines: { officecli: { path: "engines/officecli/officecli", version: "1.0.160" }, typst: { path: "engines/typst/typst", version: "0.15.1" } }`（Windows 路径带 `.exe`；当前平台没有条目的引擎不写）。
- Produces: 每个引擎目录里的 `LICENSE`（和 Node 一样随可执行文件拷贝；officecli 的 LICENSE/NOTICE 从仓库根取，typst 的从 tar 包取）。

- [ ] **Step 1: 写失败的打包测试**

在 `assistant-runtime/test/packaging.test.mjs` 仿 :192 的写法加：
- `manifest lists both engines with the pinned versions`：读 `resources/manifest.json`，断言 `manifest.engines.officecli.version === downloads.officecli.version`、`manifest.engines.typst.version === downloads.typst.version`，`path` 指向的文件存在且可执行（非 Windows 检查 mode & 0o111）。
- `officecli and typst notices name the version and the Apache license`：`notices/officecli.txt` 匹配 `/officecli 1\.0\.160/` 和 `/Apache License\s+Version 2\.0/`；`typst.txt` 同理匹配 `/typst 0\.15\.1/`。
- `engine stamp matches the pinned digests`：`resources/engines/source.json` 的 sha256 等于 json 里的值。

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm assistant:test 2>&1 | grep -E "^not ok|# fail"`
Expected: 三个新测试失败（manifest 无 engines / notices 不存在）。

- [ ] **Step 3: 实现 `prepareEngines()`**

在 `scripts/assistant-prepare.mjs`：
- 把 `cachedDownload(download)` 改成 `cachedDownload({ url, archive, sha256 })`，Node 的调用处传入现在写死的 URL。
- 新增 `async function prepareEngines()`：读 `engine-downloads.json`，对每个引擎取 `target`（:32）对应条目；没有条目就跳过并打印一行；stamp 命中且 `--version` 输出含版本号就跳过；否则下载→校验→（有 `archive` 则用 :94-104 的 tar 解压到临时目录）→拷到 `resources/engines/<name>/`→chmod 755→拷 LICENSE→写 stamp。
- 主流程在 `prepareSearchServer()`（:338）之后调用 `prepareEngines()`，保证 `signMacFiles(resources)`（:339）会覆盖它们；manifest 写入处加 `engines` 字段。
- `assistant:install`（`--install-only`）不下载引擎。

- [ ] **Step 4: 写 notices**

`notices/officecli.txt`：标题、`===` 下划线、一句说明（版本、来源 URL、Apache-2.0）、LICENSE 全文、NOTICE 全文、THIRD-PARTY-NOTICES.txt 全文（从仓库 v1.0.160 tag 取）。`notices/typst.txt`：同样格式，LICENSE 全文。格式参考 `notices/office.txt`。

- [ ] **Step 5: smoke 跑引擎**

`scripts/assistant-smoke.mjs` 的 `productionSmoke` 里加：在生产树里 `execFileSync(manifest.engines.officecli.path, ["--version"], { env: { ...minimalEnv, OFFICECLI_NO_AUTO_RESIDENT: "1", OFFICECLI_SKIP_UPDATE: "1", HOME: tmp, TMPDIR: tmp } })` 输出含 `1.0.160`；再在 tmp 里 `officecli create t.docx` + `officecli add t.docx /body --type paragraph --prop text=smoke`，断言文件存在且以 `PK` 开头；`typst --version` 输出含 `0.15.1`，并把一个只含 `= Smoke` 的 `main.typ` 编译成 PDF，断言以 `%PDF` 开头。:364-373 的 notices 列表加 `officecli.txt`、`typst.txt`。

- [ ] **Step 6: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -6 && pnpm assistant:smoke 2>&1 | tail -8`
Expected: 全部通过；smoke 打印两个引擎的版本。

- [ ] **Step 7: 提交**

```bash
git add assistant-runtime/engine-downloads.json assistant-runtime/notices/officecli.txt assistant-runtime/notices/typst.txt scripts/assistant-prepare.mjs scripts/assistant-smoke.mjs assistant-runtime/test/packaging.test.mjs
git commit -m "feat(assistant): bundle officecli and typst as document engines"
```

---

### Task 2: 签名 spike（officecli 的 entitlements）

**Files:**
- Create: `assistant-runtime/officecli-entitlements.plist`
- Modify: `scripts/assistant-prepare.mjs` `signMacFiles` :210-231（entitlements 按文件名选择）
- Create: `docs/assistant-engines.md`（引擎的来源、版本、签名要求、spike 结果、退路）

**Interfaces:**
- Produces: `officecli-entitlements.plist` 含 `com.apple.security.cs.allow-jit`、`com.apple.security.cs.allow-unsigned-executable-memory`、`com.apple.security.cs.disable-library-validation`（三项都 `true`）。typst 不带 entitlements。
- Produces: `signMacFiles` 里 `const ENTITLEMENTS = { node: "node-entitlements.plist", officecli: "officecli-entitlements.plist" }`，按 `path.basename(filename)` 查表（Windows 可执行文件名不出现在 darwin 上，不用管 `.exe`）。

- [ ] **Step 1: 改 `signMacFiles`，写 plist**

:226-230 的 `filename === node` 判断改为查表。

- [ ] **Step 2: 本地签名并运行（门槛）**

Run: `APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:prepare && codesign -dv --entitlements - assistant-runtime/resources/engines/officecli/officecli 2>&1 | grep -E "runtime|allow-jit|disable-library-validation"`
Expected: 输出含 `flags=0x10000(runtime)` 和三项 entitlements。

Run: `APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:smoke 2>&1 | tail -8`
Expected: 两个引擎在签名后仍正常运行（这是 spike 的判定）。

如果 officecli 被 killed 或报 `code signature invalid` / `library validation` 类错误：先试去掉 `disable-library-validation`，再试加 `com.apple.security.cs.allow-dyld-environment-variables`；都不行就停下，按 spec "已决定" 第 2 条改为按需安装，并向用户报告。

- [ ] **Step 3: 写 `docs/assistant-engines.md`**

内容：两个引擎是什么、版本、下载与校验方式、notices、macOS 签名要求与 spike 结果（命令和输出摘要）、公证待办（本机没有 `APPLE_ID` 等凭据，由用户在 CI 或本机导出凭据后跑一次 `scripts/release.sh` 风格的公证）、Windows 不需要额外处理、退路。

- [ ] **Step 4: 提交**

```bash
git add assistant-runtime/officecli-entitlements.plist scripts/assistant-prepare.mjs docs/assistant-engines.md
git commit -m "build(assistant): sign officecli with JIT entitlements and document the engines"
```

---

### Task 3: 宿主定位引擎并在 bootstrap 里暴露

**Files:**
- Create: `src-tauri/src/services/assistant/documents/mod.rs`、`src-tauri/src/services/assistant/documents/engines.rs`
- Modify: `src-tauri/src/services/assistant/mod.rs`（`pub mod documents;`）
- Modify: `src-tauri/src/services/assistant/host.rs`（`Runtime` 结构 :200 旁加 `engines`，或 `runtime()` :941 之后加 `engines()`；`runtime_at` :635 读 manifest 的 `engines`）
- Modify: `src-tauri/src/services/assistant/requests.rs:97-100`（bootstrap 的 `runtime` 对象加 `documents`）
- Modify: `src/lib/api/assistant.ts`（bootstrap 类型加 `runtime.documents?: { formats: string[] }`）
- Test: `engines.rs` 内 `#[cfg(test)]`；`requests.rs` 现有 bootstrap 测试

**Interfaces:**
- Produces（`documents/engines.rs`）：
  - `pub struct Engines { pub officecli: Option<PathBuf>, pub typst: Option<PathBuf> }`
  - `pub fn engines_at(base: &Path) -> Engines`：读 `base/manifest.json` 的 `engines` 映射，`path` 相对 `base`，文件不存在则该项为 `None`；manifest 缺字段或解析失败返回全 `None`，不报错。
  - `impl Engines { pub fn formats(&self) -> Vec<&'static str> }`：始终含 `md, txt, csv, html, json`；`officecli` 存在时加 `pptx, docx, xlsx`；`typst` 存在时加 `pdf`。顺序固定如上。
- Produces（host）：`pub(super) fn engines(&self) -> Engines`，用 `env.runtime_dir()` 调 `engines_at`，结果缓存在 `Runtime` 旁。
- Produces（bootstrap JSON）：`runtime.documents = { "formats": [...] }`。

- [ ] **Step 1: 写失败的测试**

`engines.rs`：
- `engines_at_reads_paths_from_manifest`：临时目录写 manifest `{"engines":{"officecli":{"path":"engines/officecli/officecli"},"typst":{"path":"engines/typst/typst"}}}` 并创建两个空文件，断言两个 `Some` 且为绝对路径。
- `engines_at_ignores_missing_files_and_absent_field`：只写 officecli 条目但不建文件 → 全 `None`；manifest 没有 `engines` → 全 `None`。
- `formats_depend_on_engines`：全 `None` → `["md","txt","csv","html","json"]`；两者都有 → 含 `pptx,docx,xlsx,pdf`。

`requests.rs`：在现有 bootstrap 测试旁加 `bootstrap_reports_document_formats`，用 `FixedEnvironment` 指向含 manifest 的临时目录，断言 `runtime.documents.formats` 含 `"pdf"`。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents::engines 2>&1 | grep -E "test result|error\[E"`
Expected: 编译错误（模块不存在）。

- [ ] **Step 3: 实现**

按上面的签名实现；`Runtime` 不改字段，`engines()` 单独缓存（`OnceLock<Engines>` 或和 `runtime()` 同样的锁），避免动 :635 的 Node 解析逻辑。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents:: 2>&1 | grep "test result" && cargo test --manifest-path src-tauri/Cargo.toml bootstrap 2>&1 | grep "test result"`
Expected: 全部 ok。`pnpm typecheck` 通过。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/assistant/documents src-tauri/src/services/assistant/mod.rs src-tauri/src/services/assistant/host.rs src-tauri/src/services/assistant/requests.rs src/lib/api/assistant.ts
git commit -m "feat(assistant): locate bundled document engines and report formats"
```

---

### Task 4: 网关探测脚本（多参考图、Responses 并行调用）

**Files:**
- Create: `scripts/assistant-gateway-probe.sh`
- Modify: `docs/assistant-engines.md`（附一节"网关探测结果"，由用户跑完后填）

**Interfaces:**
- Produces: `bash scripts/assistant-gateway-probe.sh <outdir>`，读环境变量 `OFOX_API_KEY`（没有就退出 2 并提示），`OFOX_APEX` 默认 `ofox.ai`，`PROBE_MODEL` 默认 `openai/gpt-6.1-terra`，`PROBE_IMAGE_MODEL` 默认 `openai/gpt-image-2`。
- 四个请求，响应写到 `<outdir>`，key 不打印：
  1. `/v1/images/edits` multipart，带两张 `image` 字段（用脚本内生成的 1×1 PNG），判断是 200 还是 4xx；
  2. `/v1/chat/completions` 并行工具调用（流式），统计 `delta.tool_calls[].index` 的取值集合；
  3. `/v1/chat/completions` 同样请求加 `parallel_tool_calls:false`；
  4. `/v1/responses` 并行工具调用（流式），统计 `output_index` / `item id`。
- 结尾打印一张表：每项 支持/不支持/异常。

- [ ] **Step 1: 写脚本**（只用 curl + python3 -I 解析）

- [ ] **Step 2: 本地跑一次**

Run: 由用户或主会话用钥匙串里的 key 跑（`OFOX_API_KEY=$(security find-generic-password -s ai.ofox.desktop.apikey -a opencode -w) bash scripts/assistant-gateway-probe.sh /tmp/probe`，子 agent 不要自己读钥匙串）。
Expected: 四行结果。把结论写进 `docs/assistant-engines.md`。

- [ ] **Step 3: 提交**

```bash
git add scripts/assistant-gateway-probe.sh docs/assistant-engines.md
git commit -m "chore(assistant): add gateway probe for image edits and parallel tool calls"
```

---

### Task 5: 引擎子进程运行器（Rust）

**Files:**
- Create: `src-tauri/src/services/assistant/documents/runner.rs`
- Test: 同文件 `#[cfg(test)]`（unix 用 shell 脚本充当引擎，写法参考 `host.rs:4724 stub_runtime`）

**Interfaces:**
- Consumes: `process::spawn`（`process.rs:47`）、`process::clean_environment`（:123）、`process::bounded_output`（:106）、`Child::stop`（:95）；超时模板 `tools/execute.rs:29-103`。
- Produces:
  - `pub struct EngineOutput { pub status: Option<i32>, pub stdout: String, pub stderr: String, pub timed_out: bool }`
  - `pub async fn run_engine(exe: &Path, args: &[&str], cwd: &Path, home: &Path, timeout: Duration) -> Result<EngineOutput, String>`：`clean_environment` 后设 `HOME`、`USERPROFILE`、`TMPDIR`、`TEMP`、`TMP` 为 `home`，`OFFICECLI_NO_AUTO_RESIDENT=1`、`OFFICECLI_SKIP_UPDATE=1`、`TYPST_PACKAGE_PATH` 和 `TYPST_PACKAGE_CACHE_PATH` 为 `home/typst-packages`（空目录，先创建）；stdout/stderr 各 ≤ 256 KiB；超时 `child.stop()` 并置 `timed_out`。
  - `pub const ENGINE_TIMEOUT: Duration = Duration::from_secs(60);`

- [ ] **Step 1: 写失败的测试**

- `run_engine_passes_isolated_environment`：桩脚本 `env > "$1"` 后退出 0，断言输出文件里 `HOME=` 在传入的 `home` 下、含 `OFFICECLI_SKIP_UPDATE=1`、`OFFICECLI_NO_AUTO_RESIDENT=1`、不含 `OFOX_` 开头的变量。
- `run_engine_bounds_output`：桩脚本输出 1 MiB 到 stdout，断言 `stdout.len() <= 256 * 1024`。
- `run_engine_times_out_and_kills`：桩脚本 `sleep 30`，timeout 1 s，断言 `timed_out` 且调用在 5 s 内返回。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents::runner 2>&1 | grep -E "test result|error"`

- [ ] **Step 3: 实现**

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents::runner 2>&1 | grep "test result"`
Expected: ok。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/assistant/documents/runner.rs src-tauri/src/services/assistant/documents/mod.rs
git commit -m "feat(assistant): run document engines in an isolated child process"
```

---

### Task 6: `create_document` 的宿主实现

**Files:**
- Create: `src-tauri/src/services/assistant/documents/create.rs`（参数、校验、三条管线）
- Create: `src-tauri/src/services/assistant/documents/pdf-prelude.typ`（`include_str!`）
- Modify: `src-tauri/src/services/assistant/attachments.rs`（新增 `generated_file`，仿 :534 `generated_video`）
- Modify: `src-tauri/src/services/assistant/store.rs`（新增 `outputs_dir(sid, doc_id)`，用 :1402 `private_subdir`）
- Modify: `src-tauri/src/services/assistant/host.rs:1763-1777`（路由分支）
- Modify: `src-tauri/src/services/assistant/media_tools.rs`（或 `documents/mod.rs`）：`pub fn is_document_tool(name) -> bool`
- Test: `create.rs` `#[cfg(test)]`（桩引擎）

**Interfaces:**
- Consumes: Task 3 `Engines`、Task 5 `run_engine`、`media_tools/host.rs:999 publish_generation`、`:930 save_generation`、`types.rs:12 MediaRecord`、`registry::Call`（`tools/registry.rs:72`）。
- Produces（参数，`#[serde(deny_unknown_fields)]`）：

```rust
pub struct CreateDocumentArgs { pub name: String, pub format: DocumentFormat, pub content: serde_json::Value }
pub enum DocumentFormat { Pptx, Docx, Xlsx, Pdf, Md, Txt, Csv, Html, Json }
```

  校验：`name` 去掉路径分隔符、控制字符、前导点，≤ 120 字符，扩展名必须等于 `format`（没有就补上）；`content` 序列化 ≤ 2 MiB；Office 格式 `content` 必须是数组、≤ 2000 项，每项必须有 `command`；pdf 和文本格式必须是字符串。
- Produces（结果 JSON，成功）：

```json
{ "status": "completed",
  "file": { "id": "<attachmentId>", "name": "…", "mimeType": "…", "bytes": 12345, "slides": 8 },
  "issues": [ { "severity": "warning", "message": "…", "path": "/slide[2]" } ],
  "filePath": "/abs/…/outputs/<docId>/<name>", "previewPath": "/abs/…/outputs/<docId>/preview-1.png" }
```

  `slides`/`pages`/`sheets` 从 `officecli view <file> stats --json` 取（字段名以实际输出为准，取不到就省略）；pdf 的 `pages` 由运行时用 pdfjs 补；`previewPath` 只有 pdf 有（Task 8 的 pptx 预览走 `view svg`，由运行时自己调用不了引擎，所以 pptx 的 SVG 也在这里生成：`officecli view <file> svg --start 1 > preview-1.svg`，`previewPath` 指向它）。
- Produces（结果 JSON，失败）：按 `dispatch_media_tool` 的 `refuse` 习惯返回 `Err`，错误 JSON `{ "code": "assistant_document_failed", "message": "…", "items": [ { "index": 2, "code": "…", "output": "…" } ] }`（officecli 的逐项失败）或 `{ "code": "assistant_document_failed", "message": "<typst stderr 前 4000 字>" }`。
- Produces：`attachments::generated_file(store, sid, mid, artifact: Option<&str>, file: &mut std::fs::File, name: &str, mime: &str, mut media: MediaRecord, precondition) -> Result<Attachment, String>`：magic 校验（pdf 以 `%PDF` 开头；pptx/docx/xlsx 以 `PK` 开头；文本格式不校验）、大小 ≤ `MAX_FILE_BYTES`、`FileTxn` 0600、`check_capacity`。`Attachment.details = {"format": "...", "pages"|"slides"|"sheets": n}`。
- Produces：`MediaRecord { kind: "document", model: "", params: {name, format}, status: "completed", cost: Some("0".into()), … }`（`cost` 必须是 `Some("0")`，否则 `projection.rs:626 add_generation_usage` 会把回复总费用置空）。
- 管线：
  - Office：`officecli create <name>` → 写 `batch.json` → `officecli batch <name> --input batch.json --json` → 解析 JSON；任一项失败（或 `atomicRolledBack:true`）→ 失败结果 → `officecli validate <name> --json` → `officecli view <name> issues --limit 20 --json` → `view <name> stats --json` → pptx 时 `view <name> svg --start 1`。
  - PDF：`main.typ` = `pdf-prelude.typ`（`{{LANG}}` 替换为 `uiLanguage`）+ 正文 → `typst compile --root <docDir> main.typ out.pdf` → `typst compile --root <docDir> --format png --pages 1 --ppi 96 main.typ preview-{p}.png`；编译 stderr 里的 `warning:` 行进 `issues`。
  - 文本：直接写。
- 目录：`store.outputs_dir(sid, doc_id)` = `<sid>/outputs/<docId>/`（0700），`home` 子目录 `.home`。
- 一条回复最多 3 个文档：超过返回 `assistant_document_limit`。

- [ ] **Step 1: 写失败的测试**（桩引擎脚本；`FixedEnvironment` 注入 engines 目录）

- `create_pptx_runs_batch_and_imports_attachment`：桩 officecli 按子命令分支（`create` 写一个 `PK\x03\x04…` 文件；`batch` 输出 `{"results":[{"index":0,"success":true}]}`；`validate` 输出 `{"valid":true}`；`view … issues` 输出 `{"issues":[]}`；`view … stats` 输出 `{"slides":3}`；`view … svg` 输出 `<svg/>`）。断言结果 `status:"completed"`、`file.mimeType` 为 pptx、`file.slides == 3`、会话里出现 `origin:"generated"` 的附件、`generation` 事件 `kind:"document"`、`previewPath` 存在。
- `create_pptx_reports_batch_failures_without_attachment`：桩 `batch` 输出 `{"results":[{"index":1,"success":false,"code":"invalid_path","output":"…"}],"atomicRolledBack":true}`；断言 `Err` 的 JSON 含 `items[0].index == 1`，会话附件数不变。
- `create_pdf_compiles_with_prelude_and_preview`：桩 typst：`compile` 时若参数含 `--format png` 就写 `preview-1.png`（1×1 PNG 字节），否则写 `%PDF-1.7…`；断言 `main.typ` 以 prelude 开头且含正文、结果有 `previewPath`、`issues` 为空。
- `create_pdf_returns_compiler_errors`：桩 typst 退出 1 并写 stderr `error: expected closing bracket`；断言 `Err` 的 message 含该文本，无附件。
- `create_text_writes_directly`：`format:"md"`，断言附件 mime `text/markdown`、内容一致、不调用引擎。
- `rejects_bad_names_and_oversized_content`：`name:"../x.pptx"` → 规范化为 `x.pptx`；`content` 2.1 MiB → `assistant_document_too_large`；第 4 个文档 → `assistant_document_limit`。
- `office_format_without_engine_is_refused`：engines 全 `None` 时 `format:"pptx"` → `assistant_capability_disabled`。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents::create 2>&1 | grep -E "test result|error"`

- [ ] **Step 3: 实现**

`pdf-prelude.typ` 内容（固定，可微调）：

```typst
#set page(paper: "a4", margin: (x: 2.2cm, y: 2.4cm), numbering: "1")
#set text(font: ("PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Hiragino Sans", "Yu Gothic", "Helvetica Neue", "Arial"), size: 11pt, lang: "{{LANG}}")
#set par(justify: true, leading: 0.8em)
#set heading(numbering: none)
#show heading.where(level: 1): it => block(above: 1.4em, below: 0.8em, text(size: 20pt, weight: "bold", it.body))
#show heading.where(level: 2): it => block(above: 1.2em, below: 0.6em, text(size: 15pt, weight: "bold", it.body))
#show heading.where(level: 3): it => block(above: 1em, below: 0.5em, text(size: 12.5pt, weight: "bold", it.body))
#set table(inset: 6pt, stroke: 0.5pt + luma(180))
```

路由：`host.rs:1763-1777` 在 `is_media_tool` 之前加 `if documents::is_document_tool(name) { return self.dispatch_document_tool(run, params).await }`；`dispatch_document_tool` 仿 `media_tools/host.rs:105` 做身份校验、指纹去重、取消→unknown。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml documents:: 2>&1 | grep "test result" && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -2`
Expected: ok；clippy 无警告。

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/assistant/documents src-tauri/src/services/assistant/attachments.rs src-tauri/src/services/assistant/store.rs src-tauri/src/services/assistant/host.rs src-tauri/src/services/assistant/media_tools.rs
git commit -m "feat(assistant): create documents with officecli and typst on the host"
```

---

### Task 7: 打开白名单、保存并打开

**Files:**
- Modify: `src-tauri/src/services/assistant/media_protocol.rs`（:51-61 白名单；:16 `SaveMediaRequest` 加 `#[serde(default)] pub open: bool`；:209 `save_media` 在写入成功后返回路径）
- Modify: `src-tauri/src/commands/assistant.rs:1007 assistant_save_media`（`open == true` 时 `app.opener().open_path(path, None::<String>)`，用法见 `commands/config.rs:184`）
- Modify: `src/lib/api/assistant.ts:1298 saveMedia`（请求加 `open?: boolean`）
- Test: `media_protocol.rs` `#[cfg(test)]`；`tests/lib/assistantApi.test.ts`

**Interfaces:**
- Produces: `save_media(...) -> Result<Option<PathBuf>, String>`（`None` 表示用户取消），命令层保持对渲染层返回 `bool`。
- Produces: 白名单新增 `application/pdf`、docx/pptx/xlsx 的 OOXML mime、`text/markdown`、`text/plain`、`text/csv`、`text/html`、`application/json`；大小上限文档类 25 MiB。

- [ ] **Step 1: 写失败的测试**

- `open_file_allows_generated_documents`：生成态 pdf 附件可打开；`text/x-rust` 仍拒绝。
- `save_media_returns_the_chosen_path`：pick 返回临时路径，断言返回 `Some(该路径)` 且文件内容一致。
- 命令层：`assistant_save_media_opens_only_the_saved_path`——把 opener 抽成可注入的闭包（`fn save_and_open(host, request, pick, open: impl FnOnce(&Path))`），断言 `open` 收到的路径等于 pick 返回的路径；`open:false` 时不调用。
- `assistantApi.saveMedia` 测试：`open:true` 透传到 invoke 参数。

- [ ] **Step 2: 运行，确认失败** → **Step 3: 实现** → **Step 4: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml media_protocol 2>&1 | grep "test result" && pnpm vitest run tests/lib/assistantApi.test.ts 2>&1 | tail -3`

- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/assistant/media_protocol.rs src-tauri/src/commands/assistant.rs src/lib/api/assistant.ts tests/lib/assistantApi.test.ts
git commit -m "feat(assistant): allow generated documents to be saved and opened"
```

---

### Task 8: 运行时 `create_document` 工具与缩略图

**Files:**
- Create: `assistant-runtime/src/document-tools.mjs`
- Create: `assistant-runtime/src/thumbnails.mjs`（从 `attachments.mjs:725-758 #renderPage` 抽出可复用的 `encodeThumbnail(canvas, { maxBytes })` 和 `thumbnailFromPng(bytes)` / `thumbnailFromSvg(bytes)`）
- Modify: `assistant-runtime/src/runtime.mjs`（:28-39 `READ_TOOLS` 不变；:1265 `tools()` 在 `mediaTools` 之后注册；:826 `systemPrompt` 收集 guidelines 已自动生效）
- Test: `assistant-runtime/test/document-tools.test.mjs`、`assistant-runtime/test/thumbnails.test.mjs`、`assistant-runtime/test/runtime.test.mjs`

**Interfaces:**
- Consumes: `context.params.documents = { formats: string[] }`（Task 9 由宿主下发）；`runtime.hostTool`（:1290）；`runtime.wrapTool(context, name, description, parameters, execute)`（:1309）；`AgentToolResult.content: (TextContent|ImageContent)[]`。
- Produces（`documentTools(runtime, context, { vision })`）：一个工具 `create_document`，参数：

```js
Type.Object({
  name: Type.String({ minLength: 1, maxLength: 120, description: "File name with extension" }),
  format: Type.Union(formats.map((f) => Type.Literal(f))),
  content: Type.Union([
    Type.String({ maxLength: 2_000_000 }),
    Type.Array(Type.Record(Type.String(), Type.Unknown()), { maxItems: 2000 }),
  ]),
})
```

  `execute`：校验 Office 格式用数组、其余用字符串 → `hostTool("create_document", args)` → 若 `details.previewPath` 且 `vision`：`.png` 用 `thumbnailFromPng`，`.svg` 用 `thumbnailFromSvg`（`@napi-rs/canvas` 的 `loadImage` 能读 SVG 就渲染，不能就跳过并记 stderr）；pdf 时用 pdfjs 打开 `filePath` 取 `numPages` 填 `file.pages` → 返回 `{ content: [{type:"text", text: describe(result)}, {type:"image", data, mimeType:"image/jpeg"}?], details }`，`details` 里删掉 `filePath` 和 `previewPath`。
  `describe(result)`：成功时 "Created <name> (<format>, <bytes>, <slides> slides) as attachment <id>. Issues: N. Do not repeat the file contents; tell the user what the file contains."；失败由 wrapTool 的错误路径处理。
- Produces（`promptGuidelines`，挂在该工具上）：
  1. "When the user asks for a PDF, Word, PowerPoint or Excel file, or to write/make a document, produce the file with create_document; never say you cannot attach files."
  2. "Before the first document of a format in a conversation, call use_skill on the matching ofox-office-* skill and follow it."
  3. "Reply with two or three sentences on the structure first, then call the tool; after the result, summarize the file and mention issues."
  4. "At most 3 documents per reply; retry a failed document at most twice, then explain."
- Produces（`thumbnails.mjs`）：`encodeThumbnail(canvas, { maxBytes = 200 * 1024, maxEdge = 768 })` → `{ data: base64, mimeType: "image/jpeg" } | null`（先缩放到 maxEdge，再 JPEG 85→60→40 逐级降质，超过 maxBytes 返回 null）。

- [ ] **Step 1: 写失败的测试**

`document-tools.test.mjs`（沿用 `media-tools.test.mjs:34-49` 的 fixture）：
- `registers create_document with the host formats`：`params.documents.formats = ["pdf","md"]` → schema 的 `format` 只接受这两个。
- `forwards office content as a batch array`：`execute("c1", {name:"deck.pptx", format:"pptx", content:[{command:"add",…}]})` → `calls[0]` 的 args 原样；返回文本含 "attachment"。
- `rejects string content for office formats and arrays for pdf`：返回 isError 文本含 "array" / "string"。
- `attaches a thumbnail for vision models and strips paths`：reply 带 `previewPath` 指向测试里写出的 64×64 PNG；`vision:true` → `content[1].type === "image"`；`details.previewPath === undefined`；`vision:false` → 没有 image 块。
- `thumbnail stays within budget`：2000×2000 PNG → `content[1].data` 解码后 ≤ 200 KB；整条结果 `JSON.stringify` 长度 < 1 MiB。
- `thumbnails.test.mjs`：`encodeThumbnail` 对 2000×2000 画布返回 ≤ 200 KB；对无法压到预算的（极端噪声图）返回 null。
- `runtime.test.mjs`：`system prompt tells the model to produce files with create_document`（有 `documents` 参数时含 "never say you cannot attach"）；`no create_document without the documents param`。

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm assistant:test 2>&1 | grep -E "^not ok|# fail"`

- [ ] **Step 3: 实现**（`tools()` :1277 之后：`if (params.documents) tools.push(...documentTools(this, context, { vision: model.input.includes("image") }))`）

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -6`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git add assistant-runtime/src/document-tools.mjs assistant-runtime/src/thumbnails.mjs assistant-runtime/src/runtime.mjs assistant-runtime/test/document-tools.test.mjs assistant-runtime/test/thumbnails.test.mjs assistant-runtime/test/runtime.test.mjs
git commit -m "feat(assistant): add the create_document tool with previews for vision models"
```

---

### Task 9: 宿主下发 `documents` 参数

**Files:**
- Modify: `src-tauri/src/services/assistant/requests.rs:748-802`（`params.insert("documents", {"formats": engines.formats()})`，只有 `model.tools` 为真时）
- Test: `requests.rs` 现有 run-params 测试旁

**Interfaces:**
- Produces: `params.documents = { "formats": [...] }`；引擎全缺时 formats 只有文本格式，仍然下发（文本格式不需要引擎）。

- [ ] **Step 1: 写失败的测试**：`run_params_include_document_formats`（有引擎 → 含 pdf 和 pptx）、`run_params_document_formats_without_engines`（只含 md/txt/csv/html/json）、`run_params_omit_documents_for_models_without_tools`。
- [ ] **Step 2–4**：失败 → 实现 → `cargo test --manifest-path src-tauri/Cargo.toml run_params 2>&1 | grep "test result"` 通过。
- [ ] **Step 5: 提交**

```bash
git add src-tauri/src/services/assistant/requests.rs
git commit -m "feat(assistant): pass available document formats to the runtime"
```

---

### Task 10: 文档卡片、保存并打开、i18n、starter

**Files:**
- Modify: `src/lib/api/assistant.ts`（:858-877 `AssistantGeneration` 联合加 document 分支；:1298 `saveMedia` 已在 Task 7 改）
- Modify: `src/components/assistant/GenerationCards.tsx`（:295-352 分支加 `DocumentResult`，组件仿 :372 `SpeechResult`）
- Modify: `src/components/assistant/ReplyMeta.tsx:158-190`（kind 为 document 时不显示金额，显示 "文档 ×N"）
- Modify: `src/components/assistant/AssistantPage.tsx:120-125`（starter `{ id: "deck", kind: undefined, requires: "documents" }`，:445-457 过滤时看 `bootstrap.runtime.documents?.formats.includes("pptx")`）
- Modify: `src/i18n/locales/{zh,en,ja}.json`（`assistant.generation.kinds.document`、`assistant.document.{saveAndOpen, issues, pages, slides, sheets}`、`assistant.starters.deck.{label,prompt}`）
- Test: `tests/components/AssistantGenerationResults.test.tsx`、`tests/components/AssistantPage.test.tsx`（starter 一例）

**Interfaces:**
- Consumes: `AssistantGeneration & { kind: "document"; format: string; pages?: number; slides?: number; sheets?: number; issues?: { severity: string; message: string; path?: string }[] }`（来自 `MediaRecord.params` 和 `Attachment.details` 的投影；投影字段由 `projection.rs` 现有 generation 投影透传 `params` 即可，实现者确认）。
- Produces: `DocumentResult({ item, files, disabled, onSave, onSaveAndOpen })`：类型图标（按 format）、文件名、大小、页/张/表数、issues 徽标（点开列出，最多 20 条）；按钮 **保存并打开**（主）和 **保存…**；submitting 态用 `MEDIA_PLACEHOLDER`。
- Produces: `AssistantPage` 的 `onSaveAndOpen = (file) => assistantApi.saveMedia({ …, open: true })`。

- [ ] **Step 1: 写失败的测试**

- `AssistantGenerationResults.test.tsx`：`renders a document card with counts and both buttons`（pptx、8 slides → 文本 "8"、按钮 "Save and open"、"Save…"）；`save and open passes open:true`（mock saveMedia 被调用时 `open === true`）；`shows issues badge`（2 issues → 徽标 "2"）。
- `AssistantPage.test.tsx`：`shows the deck starter only when pptx is available`。

- [ ] **Step 2–4**：失败 → 实现 → `pnpm vitest run tests/components/AssistantGenerationResults.test.tsx tests/components/AssistantPage.test.tsx 2>&1 | tail -5`，`pnpm typecheck`，`pnpm format:check` 通过。

- [ ] **Step 5: 提交**

```bash
git add src/lib/api/assistant.ts src/components/assistant/GenerationCards.tsx src/components/assistant/ReplyMeta.tsx src/components/assistant/AssistantPage.tsx src/i18n/locales/zh.json src/i18n/locales/en.json src/i18n/locales/ja.json tests/components/AssistantGenerationResults.test.tsx tests/components/AssistantPage.test.tsx
git commit -m "feat(assistant): show generated documents with save and open"
```

---

### Task 11: 随包办公技能与随包技能通道

**Files:**
- Create: `assistant-runtime/skills/ofox-office-docx/SKILL.md`、`ofox-office-pptx/SKILL.md`、`ofox-office-xlsx/SKILL.md`、`ofox-office-pdf/SKILL.md`
- Create: `assistant-runtime/notices/officecli-skills.txt`（来源与 Apache-2.0 说明）
- Modify: `src-tauri/src/services/assistant/skills.rs`（`SkillRunState` :35 加 `bundled: Vec<BundledSkill>`；`public()` :73 合并；`load_run_skill` :164 先查 bundled；`list_skills` :118 合并）
- Create: `src-tauri/src/services/assistant/skills/bundled.rs`（扫描与 frontmatter 解析）
- Modify: `src-tauri/src/services/assistant/requests.rs`（`admit_run_skills` :691 处把 bundled 并进去）
- Modify: `src/lib/api/assistant.ts:1175 AssistantSkillChoice.origin` 加 `"bundled"`；`/` 列表组件显示"内置"标签（找到渲染 origin 标签的组件）；i18n
- Test: `bundled.rs` `#[cfg(test)]`；`skills.rs` 测试；`assistant-runtime/test/skills.test.mjs`（若存在）确认 `ofox-` 名字通过 `validName`

**Interfaces:**
- Produces（`bundled.rs`）：
  - `pub struct BundledSkill { pub name: String, pub description: String, pub dir: PathBuf }`
  - `pub fn bundled_skills(runtime_dir: &Path) -> Vec<BundledSkill>`：扫描 `runtime_dir/runtime/skills/*/SKILL.md`，跳过 `local-config`，解析 frontmatter 的 `name`/`description`（复用资源库现有的 frontmatter 解析函数；没有公开的就写最小解析：`---` 之间的 `name:`/`description:`）；`name` 必须以 `ofox-` 开头且等于目录名，否则跳过并 `log::warn!`。
  - `pub fn read_bundled(skill: &BundledSkill) -> Result<Value, String>`：返回 `{ "body": <SKILL.md 全文>, "dir": <路径>, "files": [相对路径, 最多 200] }`，和 `read_verified` 的输出形状一致。
- Produces（`SkillRunState::public()`）：用户条目在前；bundled 条目只在名字不冲突时追加，形状 `{name, description, dir, mode:"auto", hasScripts:false, reviewed:true, bundled:true}`；冲突时 `log::info!` 并跳过 bundled。
- Produces（`list_skills`）：bundled 条目 `origin:"bundled"`。
- Produces（`load_run_skill(run, name, selected)`）：名字在 bundled 且不在用户条目里 → `read_bundled`。
- 技能内容：
  - `ofox-office-pptx`：改编 OfficeCLI 仓库 `skills/pptx`（用 `gh api repos/iOfficeAI/OfficeCLI/contents/skills` 找到实际目录名）：保留版式规则，命令改为 `create_document` 的 batch 数组示例（新建幻灯片、标题、要点文本框、图片、表格、备注、背景），明确 spec A2 的子集；≤ 12000 字。
  - `ofox-office-docx`、`ofox-office-xlsx`：同理改编 `word`、`excel`。
  - `ofox-office-pdf`：自写的 Typst 正文规范：只写正文，`=`/`==` 标题，列表、表格（`#table(columns: 3, [..], ..)`）、强调、分页 `#pagebreak()`、不用 `#import` 和外部包、中文排版注意事项、一个完整示例；≤ 8000 字。
  - 每个 SKILL.md 的 frontmatter：`name: ofox-office-pptx`、`description:`（≤ 150 字，说明什么时候用）。

- [ ] **Step 1: 写失败的测试**

- `bundled_skills_scans_runtime_dir_and_skips_local_config`：临时目录造 `runtime/skills/{local-config,ofox-office-pdf,bad-name}/SKILL.md`，断言只返回 `ofox-office-pdf`。
- `public_puts_user_skills_first_and_drops_colliding_bundled`：用户条目 `ofox-office-pdf` + bundled 同名 → 只有用户那条，且无 `bundled:true`。
- `load_run_skill_reads_bundled_without_proof`：host 测试（`stub_runtime` 风格）调 `dispatch_tool(&run, &call("use_skill", {name:"ofox-office-pdf"}))`，断言 body 含 SKILL.md 正文。
- `list_skills_marks_bundled_origin`。
- 运行时：`replaceSkillIndex accepts ofox- names`。

- [ ] **Step 2–4**：失败 → 实现 → `cargo test --manifest-path src-tauri/Cargo.toml skills 2>&1 | grep "test result"`、`pnpm assistant:test`、`pnpm typecheck` 通过。

- [ ] **Step 5: 两个提交**（技能清单通道与技能内容分开，便于和 1.5.1 合并）

```bash
git add src-tauri/src/services/assistant/skills.rs src-tauri/src/services/assistant/skills src-tauri/src/services/assistant/requests.rs src/lib/api/assistant.ts src/i18n/locales/*.json <标签组件> <测试>
git commit -m "feat(assistant): merge bundled skills into the run skill list"
git add assistant-runtime/skills/ofox-office-* assistant-runtime/notices/officecli-skills.txt
git commit -m "feat(assistant): bundle office authoring skills for documents"
```

---

### Task 12: 端到端

**Files:**
- Modify: `src-tauri/tests/assistant_host.rs`（新增场景；模式参考 :5851-5943 的图片样例和 :6242 的 use_skill 样例）
- Create: `src-tauri/tests/fixtures/assistant-documents/{deck.json,essay.typ,bad-batch.json}`

**Interfaces:**
- Consumes: `Reply::ToolCalls`（:51）、`Gateway::start`（:110）、`TestEnv::new`（:456）、`turn`（:715）、`of_type(env, run, "generation")`（:3198）、`tool_results`（:3207）、`host.get_session(&sid)`。
- 前置：真实 runtime 和引擎（`pnpm assistant:prepare` 已跑）；引擎缺失时像 `runtime_dir()` 一样跳过。

- [ ] **Step 1: 写场景**

- `creates_a_pptx_from_a_batch`：`gateway.reply(ToolCalls([("use_skill", {name:"ofox-office-pptx"}), ("create_document", {name:"deck.pptx", format:"pptx", content:<deck.json，8 页>})]))` 后 `Stream("done")`；断言 `generation` 事件 `kind=="document"`、附件 `origin=="generated"`、mime 为 pptx、`details.slides == 8`、文件以 `PK` 开头；`tool_results` 里 create_document 的文本含 "attachment"。
- `creates_a_pdf_and_reports_pages`：`essay.typ`（中文正文两节）；断言 mime `application/pdf`、`details.pages >= 1`、文件以 `%PDF` 开头；视觉模型变体（`test_model()` 的 `supportsImages:true` 副本）下，下一次网关请求的 messages 里出现 image 内容。
- `creates_an_xlsx`：3×3 表格 batch；断言 mime 为 xlsx。
- `rejects_an_invalid_batch_without_a_file`：`bad-batch.json`（命令 `"explode"`）；断言 tool result isError、含 "index"、会话附件数为 0。
- `rejects_a_typst_syntax_error`：正文 `#table(` 不闭合；断言错误文本含 "error"，无附件。
- 每个场景末尾 `assert_no_secret_on_disk`。

- [ ] **Step 2: 运行**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host documents 2>&1 | grep -E "test result|FAILED|panicked"`
Expected: 全部 ok（本机有引擎）。

- [ ] **Step 3: 提交**

```bash
git add src-tauri/tests/assistant_host.rs src-tauri/tests/fixtures/assistant-documents
git commit -m "test(assistant): cover document creation end to end"
```

---

### Task 13: 收尾

**Files:**
- Modify: `docs/known-limitations.md`（加一节：PPTX 预览依赖 SVG 栅格化是否可用；公证待办；Windows 未验收）
- Modify: `docs/assistant-engines.md`（补 M2 的使用说明：工具、目录、限制）
- Modify: `README.md` / `README.zh.md`（助手能力一句话加"产出 PDF/Word/PPT/Excel"）

- [ ] **Step 1: 全量检查**

Run（依次，不并行）：`pnpm typecheck && pnpm format:check && pnpm test:unit 2>&1 | tail -3`；`cargo fmt --check --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -2`；`cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | sort | uniq -c`；`pnpm assistant:test 2>&1 | tail -3`。
Expected: 全部通过。

- [ ] **Step 2: 本机真实验收（由用户或主会话用真实 key）**

`pnpm tauri dev` 启动，助手里依次说"写一个 pdf，介绍四川"、"做一个 8 页的 Transformer 介绍 ppt"、"做一张三科成绩表"、"写一份 Word 版的周报"，每个都出现卡片，保存并打开后用系统应用无告警。记录到 `docs/assistant-engines.md`。

- [ ] **Step 3: 提交**

```bash
git add docs/known-limitations.md docs/assistant-engines.md README.md README.zh.md
git commit -m "docs(assistant): document the document engines and known limitations"
```
