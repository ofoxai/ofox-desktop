# 助手：日期、上限、重试、Pi 1.5.0、技能和 MCP 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 这一轮给助手做三件事：
- 三个小改动让它更稳，同时把 Pi 升到 1.5.0；
- 能用资源库里勾给「Ofox 助手」的技能；
- 能调用资源库里勾给「Ofox 助手」的 MCP 服务。

**Architecture:**
- 「Ofox 助手」作为资源库的第 13 个技能目标、新的 MCP 目标：技能放在 `~/.ofox-desktop/assistant/skills/`，MCP 配置写在 `~/.ofox-desktop/assistant/mcp.json`。
- 主程序（Rust）每次请求把技能列表、MCP 配置和界面语言传给运行时（Node + Pi）。
- 运行时新增 4 个技能工具。打开 Pi 内置的 `mcp` 和 `tool_search` 扩展，所有服务用 deferred 方式暴露给模型。
- 所有副作用仍由 Rust 执行，包括安装技能、保存技能和审批。

**Tech Stack:**
- 运行时：Node 22，`@earendil-works/pi-*`（1.4.0 → 1.5.0），用 node:test 写测试；
- 主程序：Rust（Tauri 2）；
- 界面：React 18 + TS，用 Vitest + RTL 写测试。

**Spec:** `docs/superpowers/specs/2026-11-05-library-resources-and-assistant-skills-design.md`，第二部分。

先完成 `docs/superpowers/plans/2026-11-05-library-resources.md`，再执行本计划。

## Global Constraints

**发布安全**：1.5.0 正在发布，不能受影响。
- 只在 `/Users/yunshen/code/ofox-desktop-1.5.1` 工作，分支 `feat/assistant-1.5.1`。
- 不 checkout、不修改、不推送 `release/1.5.0` 和 `feat/assistant-media-agent`。
- 不碰 `/Users/yunshen/code/ofox-desktop-assistant-media` 的源码。
- 不打 tag。
- 只在本地提交，不推送、不开 PR。用户说「提交」才可以。

**磁盘和内存**：
- 开工前、每次 Rust 全量编译或 `pnpm tauri build` 前，先运行 `df -h /System/Volumes/Data`。可用空间低于 20 GB 时，只能删下面三处，删完再查一次。别的东西都不删：
  - `/Users/yunshen/code/ofox-desktop-assistant-media/src-tauri/target`（直接删目录，不要在那个工作区里运行命令）；
  - `/Users/yunshen/code/ofox-desktop/src-tauri/target`；
  - 本工作区的 `src-tauri/target/debug/incremental`。
- 同一时间只跑一个重任务：`cargo build`、`cargo test`、`cargo clippy`、`pnpm test:unit`、`pnpm assistant:test` 和 `pnpm tauri build` 不要并行。
- 不要在 `/Applications` 安装或启动新版 app。启动新版可能会给已绑定的工具重新签发 Ofox API key。

**开源边界**：
- 只借鉴开源项目的思路，比如 dsh（MIT）、opencode（MIT）、pi（MIT）。
- 提交信息和代码注释里不提任何闭源产品。
- 不复制任何带专有许可的技能，比如 Anthropic 的 pdf 技能。

**界面文字**：新加的界面文字都要有中、英、日三种语言，放在 `src/i18n/locales/{zh,en,ja}.json`。

**Ofox 网址**：用到 Ofox 网址时，用 `src/lib/ofoxUrls.ts` + `useOfoxApex`，不写死 `.ai` 或 `.io`。

**Windows**：
- 路径用平台 API 拼接；npx 在 Windows 上是 `npx.cmd`。
- 文件权限：在 Unix 上用 0600 / 0700；Windows 上沿用项目已有的写法。

**项目规范**：
- 用 pnpm。命令找不到时，用 corepack 缓存里的 pnpm：`node "$HOME/.cache/node/corepack/v1/pnpm/10.34.5/bin/pnpm.cjs"`。
- 提交信息用简洁的 Conventional Commits，不带生成信息。
- 每个提交都要编译通过、测试全过。
- Rust 改动要过 `cargo fmt --check` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`。
- 运行时 `src` 改动后，要先 `pnpm assistant:prepare` 再跑 `pnpm assistant:test`，以及会用到运行时的 Rust 测试。

**MCP 配置里的密钥**：`~/.ofox-desktop/assistant/mcp.json` 和每次会话的 `runtime-settings/mcp.json` 都可能含有 API key：
- 文件权限设为 0600；
- 日志、错误信息和发给模型的内容里，不能出现其中的 env 和 header 的值。

## Review Focus

以下几种情况最容易出问题，每一条都要有测试覆盖：
1. **SKILL.md 坏掉了**：没有 frontmatter、缺 name 或 description、文件超大、是二进制。助手不能崩，这个技能也不进列表。资源库里这个技能显示「无法读取」，不能悄悄消失。测试放在 Task 3。
2. **`use_skill` 读了不该读的技能**：「只能手动用」、坏掉的、不存在的技能，以及名字里带 `../` 的，都要拒绝，并返回清楚的原因。测试放在 Task 4。
3. **模型绕过用户安装技能**：`install_skill` 传入的 `resultId` 不在这一轮回答最近一次 `find_skills` 的结果里，或者这一轮根本没搜索过，都要拒绝，而且不弹审批卡片。同一个结果安装两次，只装一次。测试放在 Task 6。
4. **MCP 服务起不来**：命令不存在、没装 uv、npx 下载失败。这一条回答照常完成，模型会被告知这个服务不可用，界面上显示原因。测试放在 Task 7。
5. **含密钥的 MCP 配置**：错误信息、日志和工具结果里不能出现 env 或 header 的值。测试放在 Task 7。

---

### Task 0: 准备

- [ ] **Step 1: 确认第一份计划已完成，基线测试都通过**

Run: `cd /Users/yunshen/code/ofox-desktop-1.5.1 && git branch --show-current && git log --oneline -8 && df -h /System/Volumes/Data | tail -1`
Expected: 分支是 `feat/assistant-1.5.1`，第一份计划的几个提交都在，可用空间大于 20 GB。

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -8`
Expected: 全部通过，没有失败。

---

### Task 1: Pi 升到 1.5.0

**Files:**
- Modify: `assistant-runtime/package.json`（`@earendil-works/pi-ai`、`pi-coding-agent`、`pi-mcp` 改为 `1.5.0`；`pi-codemode` 如果在依赖里也一起改；`undici` 的 override 改为 Pi 1.5.0 需要的版本，至少 `8.12.3`）
- Modify: 运行时的 lockfile（用现有的安装脚本更新）

- [ ] **Step 1: 改版本号并安装**

先看一下 `scripts/assistant-prepare.mjs` 怎么安装运行时的依赖。然后改版本号，运行 `pnpm assistant:install`。

Run: `node -e 'for (const p of ["pi-ai","pi-coding-agent","pi-mcp"]) console.log(p, require("./assistant-runtime/node_modules/@earendil-works/"+p+"/package.json").version)'`
Expected: 三个都是 `1.5.0`。

- [ ] **Step 2: 跑运行时测试和助手的 Rust 测试**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -8 && cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host 2>&1 | grep "test result"`
Expected: 全部通过。

1.5.0 对工具参数的校验更严。如果有测试失败，原因只能是校验报错的文字变了，或者我们某个工具的参数写法不合规。前一种情况改测试的断言，后一种情况改工具定义。不要关掉校验。

- [ ] **Step 3: 提交**

```bash
git add assistant-runtime/package.json assistant-runtime/pnpm-lock.yaml package.json pnpm-lock.yaml
git commit -m "chore(assistant): Pi 1.5.0 and undici security update"
```

`git add` 时只加实际改动的 lockfile。

---

### Task 2: 日期、搜索上限、自动重试

工具调用次数的上限不改。本机配置在每个对话里都开着，普通对话用的是本机配置任务的上限（60 步），见设计文档 B1。

**Files:**
- Modify: `assistant-runtime/src/runtime.mjs`：
  - `systemPrompt()`，约第 733 行；
  - 对话请求的错误处理，约第 1452 行的 `retry` 设置附近。
- Modify: `assistant-runtime/src/search.mjs`（`MAX_SEARCHES`，第 25 行）
- Modify: `src-tauri/src/services/assistant/requests.rs`（参数里加 `uiLanguage`）
- Modify: 显示「重试中」的界面组件。先找出显示失败卡片 `RetryCard` 的地方，就在旁边加。
- Test: `assistant-runtime/test/runtime.test.mjs`、`assistant-runtime/test/search.test.mjs`、对应的 Rust 测试

**Interfaces:**
- Produces:
  - 运行时参数 `params.uiLanguage: "zh" | "en" | "ja"`。
  - 测试用的时钟覆盖 `params.now?: string`（ISO 时间），没传就用当前时间。
  - 系统提示里加一行：`Current date: 2026-11-05 (Thursday). Time zone: Asia/Shanghai. App language: zh.`
  - 运行时发给界面的事件 `{ type: "retrying", attempt: 1 | 2 }`。

- [ ] **Step 1: 写失败的测试**

运行时测试，沿用 `runtime.test.mjs` 里假模型服务的写法：
- `system prompt states the date, time zone and app language`：
  - 传入 `now: "2026-11-05T02:00:00Z"`，并把时区设为 `TZ=Asia/Shanghai`；
  - 系统提示里要包含 `Current date: 2026-11-05 (Thursday)`、`Time zone: Asia/Shanghai` 和 `App language: zh`。
- `a run allows 6 searches`（放在 `search.test.mjs`）：第 7 次搜索被拒绝。
- `retries a request that failed before any response`：
  - 假服务前 2 次返回 503，第 3 次正常；
  - 最后只有一条回答，界面收到两条 `retrying` 事件；
  - 间隔分别不少于 1 秒和 3 秒。用假的计时器测，或者把间隔做成可以注入的参数。
- `honours Retry-After up to 10 seconds on 429`：`Retry-After: 2` 就等 2 秒；`Retry-After: 60` 最多等 10 秒。
- `does not retry after a partial response`：假服务先发出一段文本，然后断开连接。不重试，照常显示失败。
- `does not retry a timeout or a 500`：不重试。

Rust 测试：`requests.rs` 的参数里，`uiLanguage` 的值和应用设置里的语言一致。

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm assistant:test 2>&1 | grep -E "^not ok|# fail"`
Expected: 上面这些新测试都失败。

- [ ] **Step 3: 实现**

- 时区用 `Intl.DateTimeFormat().resolvedOptions().timeZone` 取；星期用英文。
- 重试：如果 Pi 1.5.0 的 `retry` 设置能精确限定到这几种情况，就用它；不能的话，就在我们发请求的那一层自己实现。判断依据是有没有收到过响应：收到过任何内容，就一律不重试。
- 「本机配置」任务的上限不变（`capabilities.rs` 第 45 到 52 行）。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -8 && cargo test --manifest-path src-tauri/Cargo.toml assistant 2>&1 | grep "test result" | tail -3`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git commit -am "feat(assistant): know today's date, allow 6 searches, retry requests that never reached the model"
```

---

### Task 3: 资源库加「Ofox 助手」技能目标，以及三档开关

**Files:**
- Create: `src-tauri/src/services/library/assistant_target.rs`
- Modify: `src-tauri/src/services/library/extension_targets.rs` 或 `skill_targets.rs`（加目标 `ofox-assistant`，看哪个文件管这个列表）
- Modify: `src-tauri/src/services/library/mod.rs`、`src-tauri/src/commands/library.rs`（读取和设置档位的命令）
- Modify: `src/lib/api/library.ts`、`src/components/library/LibraryTargets.tsx`、`src/components/library/skillDistribution.ts`（「Ofox 助手」这一项的档位选择）
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `assistant_target.rs` 里的 Rust 单元测试；`tests/components/library/LibrarySkillsPage.test.tsx`

**Interfaces:**
- Produces（Rust，`assistant_target.rs`）：
  - `pub fn assistant_dir() -> PathBuf`：`get_app_config_dir()/assistant`。`get_app_config_dir()` 会遵从用户自定义的数据目录；测试时把它指向临时目录。
  - `pub fn skills_dir() -> PathBuf`：`assistant_dir()/skills`。
  - `pub fn mcp_config_path() -> PathBuf`：`assistant_dir()/mcp.json`。
  - `pub enum SkillMode { Auto, Manual }`，序列化成 `"auto"` 和 `"manual"`。
  - `pub fn skill_modes() -> Result<BTreeMap<String, SkillMode>, AppError>`：读 `assistant_dir()/skill-modes.json`。文件不存在时返回空表；表里没有的技能都按 `Auto` 处理。
  - `pub fn set_skill_mode(name: &str, mode: SkillMode) -> Result<(), AppError>`：原子写入。
  - `pub struct AssistantSkill { name, description, dir: PathBuf, mode: SkillMode, has_scripts: bool, reviewed: bool, broken: Option<String> }`。
  - `pub fn list_assistant_skills(reviewed_names: &BTreeSet<String>) -> Vec<AssistantSkill>`：
    - 扫描 `skills_dir()` 下每个 `<name>/SKILL.md`，读 frontmatter 里的 `name` 和 `description`；
    - `has_scripts`：文件夹里有 `*.py`、`*.sh`、`*.js`、`*.ts` 或 `scripts/` 时为真；
    - `reviewed`：名字在 `reviewed_names` 里时为真；
    - 读不了的技能，`broken` 写上原因。
  - `reviewed_names` 包含：
    - 我们推荐目录里的技能；
    - 热门技能快照（`src/components/library/popularSkills.json`）里 `official` 为真的技能。

    Rust 侧直接读这个 JSON（用 `include_str!` 编译进程序）。推荐目录在 TS 里，把它的技能 id 导出成一份 JSON 给 Rust 用：可以放在 `popularSkills.json` 旁边，也可以由脚本生成，二选一。
  - 来源标签：
    - 「已审核」：名字在 `reviewed_names` 里；
    - 「助手创建」：名字在 `agent-skills.json` 里；
    - 「未审核」：从 skills.sh 或 GitHub 装的其他技能；
    - 从 12 个工具的文件夹扫到的用户自己的技能，不打标签。
- Produces（界面）：
  - Tauri 命令 `library_assistant_skill_modes()` 和 `library_set_assistant_skill_mode(name, mode)`；
  - 在 `src/lib/api/library.ts` 里各包一个函数，名字照现有的写法起。
- 目标的显示名：中文「Ofox 助手」，英文「Ofox Assistant」，日文「Ofox アシスタント」。这个目标永远显示为「已安装」，不读共享的 `~/.agents/skills`（`reads_shared: false`）。

- [ ] **Step 1: 写失败的测试**

Rust：
- `assistant_target_is_listed_and_points_at_the_assistant_skills_dir`
- `skill_modes_default_to_auto_and_round_trip`
- `list_reports_a_broken_skill_instead_of_dropping_it`：分别准备缺 frontmatter、缺 description、SKILL.md 超过 1 MB、二进制文件这四种情况。每一种都要在结果里，`broken` 有值。
- `has_scripts_detects_script_files_and_scripts_folder`
- `updating_a_modified_skill_asks_first`：
  - 资源库的技能更新在 `src/components/library/SkillUpdate.tsx` 和相应的 Rust 代码里。
  - 技能文件夹的内容在安装后被改过的话，更新前要先询问，不能直接覆盖。
  - 现有逻辑如果已经这样做了，只补这条测试；没有的话，按「安装时的内容哈希和当前内容不一致」来判断。

界面：
- `the Ofox Assistant target offers auto and manual use`：勾选「Ofox 助手」后会出现档位选择，默认是「自动用」；切换到「只能手动用」时，会调用 `library_set_assistant_skill_mode`，参数是 `"manual"`。
- `adding a skill does not pre-select the Ofox Assistant target`：现有的安装选择会默认勾上所有检测到的工具（`LibrarySkillsPage.tsx` 的 `installAgents`），但「Ofox 助手」要排除在外，默认不勾。这个规则同样适用于 MCP 的添加。
- `a broken assistant skill shows why`：显示「无法读取」和具体原因。

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml assistant_target && pnpm exec vitest run tests/components/library/LibrarySkillsPage.test.tsx`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 分发到这个目标时，复用现有的「复制 / 链接技能文件夹到目标目录」逻辑。
- 「助手创建」的技能单独记在 `assistant_dir()/agent-skills.json`，里面是一个技能名称数组。本任务只需要提供读函数 `agent_created() -> BTreeSet<String>`，写入在 Task 6。

- [ ] **Step 4: 运行，确认通过**

Run: `cargo test --manifest-path src-tauri/Cargo.toml library && pnpm exec vitest run tests/components/library/`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git commit -am "feat(library): Ofox Assistant as a skill target with auto or manual use"
```

---

### Task 4: 运行时加载技能，`use_skill`

**Files:**
- Create: `assistant-runtime/src/skills.mjs`
- Modify: `assistant-runtime/src/runtime.mjs`（注册工具；在系统提示里加技能列表；`noSkills` 保持 `true`）
- Modify: `src-tauri/src/services/assistant/requests.rs`（参数里加 `skills`）
- Modify: `src-tauri/src/services/assistant/tools/policy.rs`（本机配置下，技能文件夹可以读和执行，但不能写）
- Test: `assistant-runtime/test/skills.test.mjs`（新建）；`src-tauri/tests/assistant_host.rs`

**Interfaces:**
- Consumes: Task 3 的 `list_assistant_skills`、`skill_modes`。
- Produces:
  - 运行时参数 `params.skills: Array<{ name: string; description: string; dir: string; mode: "auto" | "manual"; hasScripts: boolean; reviewed: boolean }>`。`broken` 的技能不传。
  - 系统提示的技能段：标题 `Available skills`，每行 `- <name>: <description>`。
    - 只列 `auto` 的技能；
    - 描述折成一行，最多 200 个字符；
    - 整段最多 8000 个字符，超出的不列，最后加一行 `(<n> more skills not listed; the user can pick them with /)`；
    - 这一段说明：用得上就调用 `use_skill`。
  - 工具 `use_skill({ name: string })`：
    - 返回 `{ name, dir, body, files: string[], truncated: boolean, scriptsUnavailable: boolean }`；
    - `body` 最多 60000 个字符；
    - `files` 是相对路径，最多 200 个；
    - 只能用 `auto` 的技能。`manual` 的、坏掉的、不存在的、名字里有路径分隔符或 `..` 的，返回错误 `not_available`，并附上原因；
    - 只读，不需要审批，计入每次回答的工具次数上限。
  - 不另加读文件的工具。技能里的其他文件，模型用现有的 `read` 和命令工具去读、去跑。工具说明里要写明这一点，并给出 `dir`。
  - 只有这次运行没有本机工具时，`scriptsUnavailable` 才为真。这时工具说明告诉模型：这里不能运行脚本。

- [ ] **Step 1: 写失败的测试**

运行时测试，放在 `skills.test.mjs`：
- `lists only auto skills, one line each, within 8000 characters`：传入 300 个技能，系统提示里技能段不超过 8000 个字符，最后一行写明还有多少个没列出。
- `use_skill returns the body, the folder and the files`
- `use_skill refuses manual, broken, unknown and path-like names`：分别用一个 `manual` 的技能、一个 `broken` 的技能、一个不存在的名字和 `../x` 去调用，都返回 `not_available`。
- `use_skill marks scripts unavailable only when the run has no local tools`

Rust 测试，放在 `assistant_host.rs`：
- `requests_pass_assistant_skills_without_broken_ones`
- `local_config_may_read_and_run_in_the_skills_dir_but_not_write`

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm assistant:test 2>&1 | grep -E "^not ok|# fail"`
Expected: 新测试都失败。

- [ ] **Step 3: 实现**

`skills.mjs` 只负责两件事：生成技能段，以及 `use_skill` 的定义和执行。

技能文件夹在 `~/.ofox-desktop` 下面，这个目录可能被当作受保护的目录，所以要在 `policy.rs` 里单独放开：
- 允许读取和执行 `skills_dir()` 里的文件；
- 不允许在里面写入。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -8 && cargo test --manifest-path src-tauri/Cargo.toml --test assistant_host 2>&1 | grep "test result"`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git commit -am "feat(assistant): list the Ofox Assistant skills by name and description, load one on demand"
```

---

### Task 5: 输入框用 `/` 手动选技能

**Files:**
- Modify: 助手输入框组件。用 `grep -n "<textarea" src/components/assistant/AssistantPage.tsx` 找到输入框。
- Create: `src/components/assistant/SkillPicker.tsx`
- Modify: `src/hooks/useAssistant.ts`（发送时带上选中的技能名）
- Modify: `src-tauri/src/services/assistant/requests.rs`（把技能全文附在这条用户消息前面）
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `tests/components/AssistantSkillPicker.test.tsx`（新建）；Rust 测试

**Interfaces:**
- Consumes: Task 3 的 `list_assistant_skills`。另外新增 Tauri 命令 `assistant_list_skills()`，返回 `Array<{ name, description, mode, reviewed }>`，不包含 `broken` 的技能。
- Produces:
  - 发送接口多一个可选参数 `skill?: string`。
  - 主程序把这条用户消息的正文改成：

    ```
    <skill_content name="NAME">
    BODY
    </skill_content>

    原来的消息
    ```

  - `BODY` 最多 60000 个字符。
  - 对话记录里，这条消息显示原来的文字，外加一个「技能：NAME」标签，不显示技能全文。

- [ ] **Step 1: 写失败的测试**

界面测试：
- `typing / at the start opens the skill list`：列表里能看到「自动用」和「只能手动用」的技能，没审核过的标「未审核」。
- `choosing a skill shows a chip and sends its name`：选中后输入框里出现「技能：name」标签；发送时调用的参数里有 `skill: "name"`；按 Backspace 能删掉这个标签。
- `/ in the middle of text does not open the list`

Rust 测试：
- `a picked skill is prepended to the user message`
- `an unknown picked skill is refused with a clear error`

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm exec vitest run tests/components/AssistantSkillPicker.test.tsx`
Expected: FAIL。

- [ ] **Step 3: 实现**

- 列表的样式和交互，参考输入框现有的「+」菜单（`ComposerPlus.tsx`）。
- 方向键选择，回车确认，Esc 关闭。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm exec vitest run tests/components/ && cargo test --manifest-path src-tauri/Cargo.toml assistant 2>&1 | grep "test result" | tail -3`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git commit -am "feat(assistant): pick a skill with / in the composer"
```

---

### Task 6: 对话里找技能、装技能、存成技能

**Files:**
- Modify: `assistant-runtime/src/skills.mjs`（`find_skills`、`install_skill`、`save_skill`）
- Create: `src-tauri/src/services/assistant/skill_tools.rs`（搜索、记住这一轮的搜索结果、安装、保存）
- Modify: `src-tauri/src/services/assistant/mod.rs`、`src-tauri/src/services/assistant/tools/approval.rs`（新增两种审批类型）
- Create: `src/components/assistant/SkillResultCards.tsx`（只读的结果卡片）
- Modify: 现有的审批卡片组件（`src/components/assistant/AssistantLocalConfig.tsx` 里），加入「安装技能」和「保存技能」两种
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `assistant-runtime/test/skills.test.mjs`；`skill_tools.rs` 里的 Rust 测试；`tests/components/AssistantSkillCards.test.tsx`（新建）

**Interfaces:**
- Consumes:
  - Task 3 的 `reviewed_names`；
  - 热门技能快照 `popularSkills.json`；
  - 现有的 skills.sh 搜索 `search_skills_sh`（`src-tauri/src/services/skill.rs`）；
  - 现有的技能安装流程；
  - 现有的审批机制（`tools/approval.rs`：卡片、120 秒超时）；
  - Task 3 的 `set_skill_mode` 和 `agent_created`。
- Produces（三个工具都由主程序执行，主程序和运行时之间的调用照媒体工具的写法）：
  - 工具 `find_skills({ query: string })`：
    - 在推荐技能、热门快照、skills.sh 搜索这三处找，返回最多 8 条 `{ resultId, name, source, description, installs?, reviewed }`；
    - 主程序把结果记在这一轮回答里，作为「最近一次搜索结果」，下一次搜索会替换掉；
    - 对话里显示 `SkillResultCards`：只读，没有按钮；
    - 只读，不需要审批。
  - 工具 `install_skill({ resultId: string })`：
    - `resultId` 不在这一轮最近一次的结果里时，返回错误 `not_in_results`，并且不弹卡片；
    - 校验通过后，弹出审批卡片「安装技能」，内容有名称、来源、安装量、是否审核过，按钮是「安装」和「取消」；
    - 用户点「安装」：走现有安装流程，分发到 `ofox-assistant`，档位设为 `Auto`。返回 `{ installed: true, name }`，模型接着就能 `use_skill`；
    - 用户点「取消」或超时：返回 `{ installed: false }`；
    - 同一个 `resultId` 已经装过的，直接返回 `{ installed: true, already: true }`，不再弹卡片。
  - 工具 `save_skill({ name: string; description: string; body: string })`：
    - `name` 必须是小写字母、数字和连字符组成的 kebab-case，最长 64 个字符；
    - `description` 最多 500 个字符，`body` 最多 60000 个字符；
    - 同名技能存在、但不在 `agent-skills.json` 里时，返回错误 `not_agent_created`，并且不弹卡片；
    - 弹出审批卡片「保存技能」：可以预览名称、描述和正文（按 Markdown 渲染），按钮是「保存」和「取消」；
    - 用户点「保存」：写入 `skills_dir()/<name>/SKILL.md`，名字记进 `agent-skills.json`，档位设为 `Auto`，并登记到资源库，来源为「助手创建」。
  - 「安装」和「保存」两种审批，在任何权限模式下都要用户点击，全自动模式也一样。

- [ ] **Step 1: 写失败的测试**

Rust 测试，放在 `skill_tools.rs`：
- `install_accepts_only_ids_from_the_latest_search`：
  - 四种情况都返回 `not_in_results`，而且不会产生审批请求：上一次搜索的 id、伪造的 id、这一轮没搜索过、换了一轮回答。
- `installing_twice_installs_once`
- `install_waits_for_approval_even_in_full_auto`
- `install_enables_the_skill_for_the_assistant_in_auto_mode`
- `save_refuses_a_name_that_is_not_agent_created`
- `save_writes_skill_md_and_records_it_as_agent_created`
- `save_validates_name_and_sizes`

运行时测试：
- `find_skills, install_skill and save_skill are host tools that count toward the tool limit`

界面测试，放在 `AssistantSkillCards.test.tsx`：
- `search results are shown as read-only cards`
- `the install approval shows source and review state, and installs only on click`
- `the save approval previews the skill and saves only on click`

- [ ] **Step 2: 运行，确认失败**

Run: `cargo test --manifest-path src-tauri/Cargo.toml skill_tools && pnpm exec vitest run tests/components/AssistantSkillCards.test.tsx`
Expected: FAIL。

- [ ] **Step 3: 实现**

主程序和运行时之间的工具调用，照 `assistant-runtime/src/media-tools.mjs` 和 `src-tauri/src/services/assistant/media_tools/host.rs` 现有的写法来。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -6 && cargo test --manifest-path src-tauri/Cargo.toml assistant 2>&1 | grep "test result" | tail -3 && pnpm exec vitest run tests/components/`
Expected: 全部通过。

- [ ] **Step 5: 提交**

```bash
git commit -am "feat(assistant): find skills in chat, install or save one only after the user approves"
```

---

### Task 7: MCP 接进助手

**Files:**
- Modify: `src-tauri/src/services/library/assistant_target.rs`、`mcp_targets.rs`、`extension_targets.rs`（MCP 目标 `ofox-assistant`，写入 `mcp_config_path()`）
- Modify: `src/components/library/LibraryTargets.tsx`（需要浏览器登录的服务不能勾「Ofox 助手」，并显示原因）
- Modify: `src-tauri/src/services/assistant/requests.rs`（参数里加 `mcp`）
- Create: `assistant-runtime/src/mcp-config.mjs`
- Modify: `assistant-runtime/src/runtime.mjs`：
  - `disabledBuiltinExtensions` 去掉 `mcp` 和 `tool_search`，保留 `codemode`；
  - 在 `tool_call` 钩子里给 MCP 调用加权限判断。
- Modify: 回答里显示步骤的组件（用 `grep -rn "toolName\|tool_call" src/components/assistant` 找），给 MCP 调用配服务图标
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `assistant-runtime/test/mcp.test.mjs`（新建）；Rust 测试；`tests/components/library/LibraryMcpTable.test.tsx`

**Interfaces:**
- Consumes:
  - Task 3 的 `mcp_config_path()`；
  - 第一份计划里的 `signIn` 字段：
    - 界面上，`signIn` 为真的推荐服务不能勾「Ofox 助手」。
    - Rust 侧在 `assistant_target.rs` 里维护一份需要浏览器登录的主机名列表：`SIGN_IN_HOSTS`，取自第一份计划加进来的那 8 个 `signIn` 服务的 URL 主机名。写入助手的 MCP 配置时，用这份列表再拦一次。
    - 不能用「只有 url、没有 header」来判断是否需要登录：context7、deepwiki、microsoft-learn 这类公开服务也是这种配置，但不需要登录。
    - 用户自己加的、不认识的远程服务，允许勾选。如果它其实需要浏览器登录，连接会失败，按 Review Focus 第 4 条处理，在界面上显示原因。
  - Task 1 的图标规则。
- Produces：
  - `~/.ofox-desktop/assistant/mcp.json`：格式是 `{ "mcpServers": { "<name>": { command, args, env } | { url, headers } } }`，文件权限 0600。
  - 运行时参数 `params.mcp: { servers: Record<string, object>, npxPath?: string, uvAvailable: boolean, npmRegistry?: string } | null`。
    - `npxPath`：主程序在电脑上找到的 npx 的完整路径（Windows 上是 `npx.cmd`）。找的方式沿用 app 找工具时用的那套（会看 fnm、nvm 等）。找不到就不传。
    - 助手自带的 Node 只有 `node`，没有 npm，这一轮不往安装包里加。
    - `npmRegistry`：如果 `commands/node_requirement.rs` 或 `commands/windows_install.rs` 里给工具安装选镜像的逻辑能直接复用，就传它选出的地址；不好复用就不传。
  - `mcp-config.mjs` 的函数 `writeSessionMcpConfig(agentDir, mcp): { written: string[]; unavailable: Array<{ name, reason: "needs_uv" | "needs_node" }> }`：
    - 给每个服务加上 `exposure: "deferred"`；
    - 命令是 `npx` 时：有 `npxPath` 就换成它；有 `npmRegistry` 就设上 `env.npm_config_registry`；没有 `npxPath`，原因记为 `needs_node`；
    - 不可用的服务不写进配置，原因放进 `unavailable`；
    - 写入的文件权限 0600。
  - 系统提示里列出不可用的服务及原因，界面也显示出来。
  - MCP 调用的权限，在 `tool_call` 钩子里判断工具名是否以 `mcp__` 开头：
    - 全自动模式：直接放行；
    - 默认模式和自动编辑模式：服务声明了 `readOnlyHint: true` 的直接放行；其他的走现有审批，可以选「本次 / 本会话 / 拒绝」；
    - MCP 调用计入工具次数上限。
  - 错误信息、日志和工具结果里，env 和 header 的值都要换成 `***`。

- [ ] **Step 1: 写失败的测试**

运行时测试，放在 `mcp.test.mjs`。用一个本地假的 stdio MCP 服务：在测试的 fixture 里写一个小的 node 脚本，提供一个 `readOnlyHint` 工具和一个写工具。
- `servers are written as deferred and reachable through tool_search`
- `read-only tools run without approval in default mode`
- `write tools ask for approval in default and autoEdit, not in fullAuto`
- `a server that cannot start does not break the reply`：命令不存在时，这条回答照常完成，模型会被告知这个服务不可用。
- `uvx without uv is reported as needs_uv and not written`
- `npx uses the given npx path and registry, and without one is reported as needs_node`：用一个假的 `npxPath` 测试。
- `secrets never appear in errors or logs`：配置的 env 里写 `API_KEY=sk-test-123`，故意让服务启动失败，所有输出里都不能出现 `sk-test-123`。

Rust 测试：
- `assistant_mcp_target_writes_mcpServers_with_0600`
- `sign_in_servers_cannot_target_the_assistant`
- `requests_pass_mcp_only_when_servers_exist`

界面测试：
- `the Ofox Assistant MCP target is disabled for sign-in servers with the reason`

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm assistant:test 2>&1 | grep -E "^not ok|# fail" ; cargo test --manifest-path src-tauri/Cargo.toml assistant_mcp`
Expected: 新测试都失败。

- [ ] **Step 3: 实现**

- Pi 的 MCP 配置格式和暴露方式，看 `assistant-runtime/node_modules/@earendil-works/pi-coding-agent/docs/mcp.md`，以及 SDK 示例 `examples/sdk/14-codemode-mcp.ts`（在 pi 的包或者 GitHub 上）。
- 现有的搜索 MCP 服务（`search.mjs`）不能受影响。

- [ ] **Step 4: 量一下要等多久**

用一个真实的 npx 服务（比如 `@modelcontextprotocol/server-sequential-thinking`），在一条消息里第一次调用它，记下从发出请求到工具返回的时间。冷启动量一次，接着再量一次。

Expected：把两个数字记下来，写进最终报告。这一轮不做常驻。

- [ ] **Step 5: 运行，确认通过**

Run: `pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -6 && cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | tail -5 && pnpm exec vitest run tests/components/`
Expected: 全部通过。

- [ ] **Step 6: 提交**

```bash
git commit -am "feat(assistant): call MCP servers chosen for the Ofox Assistant, ask before non-read-only calls"
```

---

### Task 8: 整体验收和报告

- [ ] **Step 1: 全量检查**

Run: `pnpm typecheck && pnpm format:check && pnpm test:unit 2>&1 | tail -4 && cargo fmt --check --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -2 && cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep -E "test result|FAILED" | tail -25 && pnpm assistant:prepare && pnpm assistant:test 2>&1 | tail -8`
Expected: 全部通过，clippy 没有警告。

开始前先看磁盘，可用空间低于 20 GB 时，按 Global Constraints 的规则清理。

- [ ] **Step 2: 在浏览器里看效果**

用第一份计划 Task 5 搭好的测试页，模拟 IPC 和对话事件，检查：
- `/` 技能列表；
- 「技能：名称」标签；
- 找技能的结果卡片，以及装完显示「已安装」；
- 存成技能的确认卡片；
- MCP 调用那一步的图标和审批卡片；
- 「重试中」的显示。

窄窗口、宽窗口、亮色、暗色都看。截图存到 `.harness-shots/`。

- [ ] **Step 3: 写报告（中文）**

报告里写：
- 每个提交的 hash 和一句话说明；
- 各项测试的数量；
- MCP 冷启动和第二次调用的耗时；
- npm 的下载源是不是复用了 app 现有的镜像选择；
- 第一份计划里没加进去的 MCP 和原因；
- 需要用户在本机用真实 key 试的事项：
  - 技能自动使用；
  - `/` 手动选技能；
  - 找技能并安装；
  - 存成技能；
  - 只读的 MCP 调用直接跑；
  - 写操作的 MCP 调用要审批；
  - 今天的日期是否正确；
- 还有没解决的问题。

推送和 Windows 自动测试，要等用户说「提交」之后再做：推送分支 `feat/assistant-1.5.1`，它符合 `feat/assistant-*` 的规则，会自动触发 Windows Probe。
