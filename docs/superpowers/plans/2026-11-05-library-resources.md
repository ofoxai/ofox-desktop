# 资源库补资源和图标 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 资源库补充资源，并给每张卡片配上图标：
- MCP 推荐约 26 个；
- 指令模板 12 个；
- 技能页一打开就有热门技能。

**Architecture:**
- 推荐数据仍放在 `src/components/library/libraryRecommendations.ts`。
- 热门技能是一份打包进 app 的 JSON 快照，由 `scripts/` 下的脚本生成。
- 图标都打包进 `src/assets/library-logos/`，由 `LibraryResourceIcon` 统一渲染，没有图标时显示首字母方块。
- 后端 `display.rs` 负责给已安装的资源匹配图标。

**Tech Stack:** React 18 + TypeScript、Tailwind 3.4、lucide-react、`@lobehub/icons-static-svg`、Vitest + RTL；Rust（Tauri 2）。

**Spec:** `docs/superpowers/specs/2026-11-05-library-resources-and-assistant-skills-design.md`，第一部分。

本计划先执行，完成后再执行 `docs/superpowers/plans/2026-11-05-assistant-skills-mcp.md`。

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
- 同一时间只跑一个重任务：`cargo build`、`cargo test`、`cargo clippy`、`pnpm test:unit` 和 `pnpm tauri build` 不要并行。
- 不要在 `/Applications` 安装或启动新版 app。启动新版可能会给已绑定的工具重新签发 Ofox API key。

**开源边界**：
- magpie（MIT，© 2026 yetone）的数据可以用，在 `src/assets/library-logos/NOTICE.md` 和每个新数据文件开头注明出处。
- 其他闭源产品不提名字，不照搬代码。

**图标**：只用打包进 app 的文件，运行时不联网获取图标。

**界面文字**：新加的界面文字都要有中、英、日三种语言，放在 `src/i18n/locales/{zh,en,ja}.json`。

**项目规范**：
- 用 pnpm。命令找不到时，用 corepack 缓存里的 pnpm：`node "$HOME/.cache/node/corepack/v1/pnpm/10.34.5/bin/pnpm.cjs"`。
- 提交信息用简洁的 Conventional Commits，不带生成信息。
- 每个提交都要编译通过、测试全过。
- Rust 改动要过 `cargo fmt --check` 和 `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`。

**格式化**：只对本次改动的文件跑 Prettier，不要对整个 `tests/` 目录跑 `prettier --write`，那会改动很多无关文件。

## Review Focus

以下几种情况最容易出问题，每一条都要有测试覆盖：
1. **暗色模式下的图标**：黑色 logo 在暗色背景下要能看清（沿用现有的 `invert` 处理）；首字母方块在两种模式下文字和背景都要有足够对比度。测试放在 Task 1。
2. **名字是空字符串、表情或中文时的首字母方块**：不能崩溃。空名字显示「?」；中文显示第一个字。测试放在 Task 1。
3. **用户手动加的 MCP**：命令和 URL 都不认识时，显示首字母方块，不能误配成某个品牌的 logo。测试放在 Task 2 的 Rust 测试里。
4. **热门技能快照**：同一仓库最多 4 个，最多 60 个，按安装量从高到低排序。快照里缺描述的条目，卡片不显示「undefined」。测试放在 Task 4。
5. **安装量格式**：999、1000、954000、1234567、0 都要显示正确：「999」「1K」「954K」「1.2M」「0」。测试放在 Task 4。

---

### Task 0: 准备

- [ ] **Step 1: 确认工作区和分支**

Run: `cd /Users/yunshen/code/ofox-desktop-1.5.1 && git branch --show-current && git log --oneline -1 && df -h /System/Volumes/Data | tail -1`
Expected: `feat/assistant-1.5.1`，HEAD 是 `2d7886ac chore(release): 1.5.0`（或者是本计划文档的提交）；可用空间大于 20 GB。

- [ ] **Step 2: 安装依赖，跑一遍基线**

Run: `pnpm install --frozen-lockfile && pnpm typecheck && pnpm test:unit 2>&1 | tail -4`
Expected: 全部通过。基线是 236 个测试文件、2879 个用例。

---

### Task 1: 通用图标组件

**Files:**
- Modify: `src/components/library/LibraryResourceIcon.tsx`
- Test: `tests/components/library/LibraryResourceIcon.test.tsx`（新建）

**Interfaces:**
- Produces:
  - `LibraryResourceIcon({ recipeId?: string; kind: LibraryKind; name?: string; className?: string })`：
    - 按 `recipeId` 能找到 logo 时显示 logo；
    - 是指令模板、并且在 `TEMPLATE_ICONS` 里时，显示带颜色的 lucide 图标方块；
    - 否则，有 `name` 就显示首字母方块；
    - 都没有时，显示原来的 Plug、Puzzle 或 FileText 图标。
  - `export function letterTile(name: string): { letter: string; className: string }`：
    - `letter` 是名字的第一个字符，英文转大写；名字是空字符串或只有空格时为 `"?"`。
    - `className` 从一组固定的 Tailwind 背景和文字配色里选一组。选法是对名字做一个简单哈希再对配色数量取模，同一个名字永远得到同一组。
  - `export const TEMPLATE_ICONS: Record<string, { icon: LucideIcon; className: string }>`：
    - key 是指令模板的 id，Task 3 会往里加；
    - 本任务先放入现有的 3 个：`coding` 用 `Code2`，`chinese` 用 `Languages`，`review` 用 `ScanSearch`。
    - 要先在 `libraryRecommendations.ts` 里核对这 3 个模板的实际 id。

- [ ] **Step 1: 写失败的测试**

```tsx
it("draws the same letter tile for the same name", () => {
  expect(letterTile("sentry")).toEqual(letterTile("sentry"));
  expect(letterTile("sentry").letter).toBe("S");
});
it("uses ? for a blank name and the first character for Chinese", () => {
  expect(letterTile("  ").letter).toBe("?");
  expect(letterTile("资源").letter).toBe("资");
});
it("shows a letter tile for an unknown MCP with a name", () => {
  render(<LibraryResourceIcon kind="mcp" name="my-server" />);
  expect(screen.getByText("M")).toBeInTheDocument();
});
it("shows the template icon for a known instruction template", () => {
  const { container } = render(
    <LibraryResourceIcon kind="instructions" recipeId="review" />,
  );
  expect(container.querySelector("svg")).not.toBeNull();
  expect(container.firstElementChild?.className).toContain(
    TEMPLATE_ICONS.review.className.split(" ")[0],
  );
});
it("keeps a known logo", () => {
  const { container } = render(
    <LibraryResourceIcon kind="mcp" recipeId="context7" name="Context7" />,
  );
  expect(container.querySelector("img")).not.toBeNull();
});
```

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm exec vitest run tests/components/library/LibraryResourceIcon.test.tsx`
Expected: FAIL，报 `letterTile` / `TEMPLATE_ICONS` 未导出。

- [ ] **Step 3: 实现**

- 实现上面的 Interfaces。
- 配色至少 8 组，只用项目已有的 Tailwind 色值。暗色模式要加 `dark:` 变体，保证对比度。
- 调用 `LibraryResourceIcon` 的地方要把资源名称传进 `name`。用 `grep -rn "LibraryResourceIcon" src` 找到所有调用处。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm exec vitest run tests/components/library/`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/components/library tests/components/library/LibraryResourceIcon.test.tsx
git commit -m "feat(library): every card gets an icon, with a letter tile when no logo is known"
```

---

### Task 2: MCP 推荐补到约 26 个

**Files:**
- Modify: `src/components/library/libraryRecommendations.ts`（加条目、加 `signIn` 字段、加分类 `search`）
- Modify: `src/components/library/LibraryDiscover.tsx`（卡片上的标记）
- Modify: `src/components/library/LibraryResourceIcon.tsx`（`RECIPE_LOGOS` 加新的 id）
- Create: `src/assets/library-logos/<id>.png`（lobehub 里没有的品牌）
- Modify: `src/assets/library-logos/NOTICE.md`
- Modify: `src-tauri/src/services/library/display.rs`（`mcp_display` 的 URL 主机名和 npx/uvx 包名匹配）
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `tests/components/library/libraryRecommendations.test.ts`（新建），`display.rs` 里的 Rust 单元测试

**Interfaces:**
- Consumes: Task 1 的 `LibraryResourceIcon`。
- Produces:
  - `RecommendationBase.signIn?: boolean`：为真时，卡片显示「需登录」。
  - `export function needsApiKey(r: LibraryRecommendation): boolean`：MCP 条目的 `initialDraft.environment` 或 `initialDraft.headers` 里有值为空的项时返回真，卡片显示「需要 API key」。
  - `LibraryRecommendationCategory` 增加 `"search"`。
  - Plan 2 的 Task 6 会用 `signIn` 判断能不能勾选「Ofox 助手」。

- [ ] **Step 1: 逐个确认 18 个服务还能用**

数据源：`/Users/yunshen/code/magpie/internal/library/market.go` 的 `featured`（第 93 到 153 行）和 `homepages`（第 193 到 209 行）。

18 个 id：exa、memory、sequential-thinking、git、time、sentry、linear、notion、atlassian、supabase、stripe、vercel、neon、cloudflare-docs、huggingface、brave-search、firecrawl、tavily。

确认方法：
- npx 包：运行 `npm view <pkg> version`。
- uvx 包：运行 `curl -s -o /dev/null -w '%{http_code}' https://pypi.org/pypi/<pkg>/json`。
- 远程服务：运行 `curl -s -o /dev/null -w '%{http_code}' <url>`。返回 401、405、406 也算能用；404 或 DNS 失败算不能用。

Expected：把每个服务的结果记下来。不能用的就不加，在本任务的提交信息里列出名字。

- [ ] **Step 2: 写失败的测试**

```ts
const mcp = LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "mcp");
it("ships the verified MCP servers", () => {
  expect(mcp.length).toBeGreaterThanOrEqual(24); // 8 + the ones that passed Step 1
  for (const id of ["exa", "notion", "linear", "tavily"])
    expect(mcp.some((r) => r.id === id)).toBe(true); // drop an id here only if Step 1 dropped it
});
it("gives every MCP a logo and translated text", () => {
  for (const r of mcp) {
    expect(hasRecipeLogo(r.id)).toBe(true);
    for (const lng of [zh, en, ja])
      expect(get(lng, r.descriptionKey)).toBeTruthy();
  }
});
it("marks browser sign-in and API key servers", () => {
  expect(mcp.find((r) => r.id === "notion")?.signIn).toBe(true);
  expect(needsApiKey(mcp.find((r) => r.id === "tavily")!)).toBe(true);
  expect(needsApiKey(mcp.find((r) => r.id === "fetch")!)).toBe(false);
});
it("puts search services in the search category", () => {
  expect(mcp.find((r) => r.id === "exa")?.category).toBe("search");
});
```

`hasRecipeLogo(id: string): boolean` 从 `LibraryResourceIcon.tsx` 导出，判断 `RECIPE_LOGOS` 里有没有这个 id。`get` 是测试里按点分隔的 key 去取 i18n 文本的小函数。

在 `display.rs` 加 Rust 测试：
- `mcp_display_matches_new_vendor_hosts`：notion、linear、sentry 等远程服务的主机名，以及 exa 等的 npx 包名，都返回对应的 icon id。
- `mcp_display_keeps_unknown_servers_plain`：`{"command":"node","args":["./server.js"]}` 和 `{"url":"https://example.com/mcp"}` 的 icon 都是 `None`。

- [ ] **Step 3: 运行，确认失败**

Run: `pnpm exec vitest run tests/components/library/libraryRecommendations.test.ts`
Expected: FAIL。

Run: `cargo test --manifest-path src-tauri/Cargo.toml mcp_display`
Expected: 第一个测试 FAIL。

- [ ] **Step 4: 实现**

**条目**：
- 按现有的 MCP 条目格式写。
- 描述：中文、英文用 magpie 的（magpie 的中文在 `/Users/yunshen/code/magpie/internal/gui/assets/i18n.js` 第 3482 行附近），日文自己翻。
- magpie 的 `Inputs` 放进 `initialDraft` 的 `environment` / `headers`：名字照抄，值留空。
- `TARGETS` 沿用现有的写法。

**图标**：
- 先看 `node_modules/@lobehub/icons-static-svg/icons/` 里有没有对应文件，有就用。
- 没有的，运行 `curl -sL 'https://github.com/<org>.png?size=96' -o src/assets/library-logos/<id>.png` 下载一次。用 `file` 确认下载下来的是 PNG。
- 在 `NOTICE.md` 里按现有格式，逐个记下来源。

**卡片标记**：在卡片标题下方的那一行加「需登录」「需要 API key」两个小标签，样式和 magpie 的「远程 · 需登录」类似。

**magpie 出处**：`libraryRecommendations.ts` 的新条目区域开头加一行注释，注明来自 magpie（MIT）。

- [ ] **Step 5: 运行，确认通过**

Run: `pnpm exec vitest run tests/components/library/ && cargo test --manifest-path src-tauri/Cargo.toml library::display`
Expected: 全部 PASS。

- [ ] **Step 6: 提交**

```bash
git add -A src/components/library src/assets/library-logos src/i18n src-tauri/src/services/library/display.rs tests/components/library
git commit -m "feat(library): recommend 18 more MCP servers with logos and sign-in/API key badges"
```

---

### Task 3: 指令模板补到 12 个

**Files:**
- Modify: `src/components/library/libraryRecommendations.ts`
- Modify: `src/components/library/LibraryResourceIcon.tsx`（往 `TEMPLATE_ICONS` 加新的 id）
- Modify: `src/i18n/locales/{zh,en,ja}.json`（标题和描述）
- Test: `tests/components/library/libraryRecommendations.test.ts`

**Interfaces:**
- Consumes: Task 1 的 `TEMPLATE_ICONS`。
- Produces: 9 个新模板，id 为 `tdd`、`debugging`、`git-commits`、`frontend`、`docs-writing`、`security`、`refactor`、`plan-first`、`concise`。

每个模板的名称和内容要点，照设计文档 A2 的表格。图标按下表：

| id | lucide 图标 |
|---|---|
| tdd | `FlaskConical` |
| debugging | `Bug` |
| git-commits | `GitCommitHorizontal` |
| frontend | `LayoutTemplate` |
| docs-writing | `BookOpen` |
| security | `ShieldCheck` |
| refactor | `Shuffle` |
| plan-first | `ListChecks` |
| concise | `Zap` |

- [ ] **Step 1: 写失败的测试**

```ts
const templates = LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "instructions");
it("ships 12 instruction templates", () => expect(templates).toHaveLength(12));
it("writes every template in three languages, 10 to 25 lines each", () => {
  for (const t of templates)
    for (const lng of ["zh", "en", "ja"] as const) {
      const lines = t.contentByLanguage[lng].trim().split("\n").length;
      expect(lines).toBeGreaterThanOrEqual(10);
      expect(lines).toBeLessThanOrEqual(25);
    }
});
it("gives every template its own icon", () => {
  for (const t of templates) expect(TEMPLATE_ICONS[t.id]).toBeDefined();
});
```

现有 3 个模板如果不足 10 行，可以补充到 10 行，但不要改变它们原来的意思。

- [ ] **Step 2: 运行，确认失败**

Run: `pnpm exec vitest run tests/components/library/libraryRecommendations.test.ts`
Expected: FAIL，提示只有 3 个模板。

- [ ] **Step 3: 实现**

- 按设计文档 A2 写 9 个模板，每个都写中、英、日三份正文。
- 正文是写给 AI 编程工具的规则，比如「先写失败的测试…」。语气和现有的 3 个一致。

- [ ] **Step 4: 运行，确认通过**

Run: `pnpm exec vitest run tests/components/library/`
Expected: 全部 PASS。

- [ ] **Step 5: 提交**

```bash
git add src/components/library src/i18n tests/components/library
git commit -m "feat(library): nine more instruction templates, each with its own icon"
```

---

### Task 4: 技能页一打开就有热门技能

**Files:**
- Create: `scripts/build-popular-skills.mjs`
- Create: `src/components/library/popularSkills.json`
- Create: `src/assets/library-logos/owners/<owner>.png`
- Create: `src/components/library/popularSkills.ts`
- Modify: `src/components/library/LibrarySkillDiscover.tsx`
- Modify: `src/assets/library-logos/NOTICE.md`
- Modify: `src/i18n/locales/{zh,en,ja}.json`
- Test: `tests/components/library/popularSkills.test.ts`（新建）、`tests/components/library/LibrarySkillDiscover.test.tsx`

**Interfaces:**
- Produces:
  - `popularSkills.json`：开头是 `{ "source": "magpie market_skills.json (MIT, yetone)", "generatedAt": "<ISO>", "skills": PopularSkill[] }`。
  - `type PopularSkill = { source: string /* owner/repo */; skillId: string; name: string; installs: number; official: boolean; description?: string }`。
  - `export function formatInstalls(n: number): string`：
    - 小于 1000 显示原数字；
    - 小于 1,000,000 显示「K」，保留一位小数，末尾的 .0 去掉；
    - 1,000,000 及以上显示「M」，同样保留一位小数。
  - `export function ownerAvatar(source: string): string | undefined`：按 owner 找打包好的头像，找不到返回 `undefined`，卡片就用首字母方块。
  - `export const POPULAR_SKILLS: PopularSkill[]`：读取 JSON，按安装量从高到低排序。

- [ ] **Step 1: 写生成快照的脚本**

`scripts/build-popular-skills.mjs` 做这几件事：
1. 读取 `/Users/yunshen/code/magpie/internal/library/market_skills.json`。
2. 每个 `source` 最多留 4 个，按 `installs` 从高到低取前 60 个。
3. 逐个请求 `https://skills.sh/<source>/<skillId>`，取 `<meta name="description">` 的内容作为英文描述。最多 6 个请求同时进行；失败的条目不填描述。
4. 写出 `src/components/library/popularSkills.json`。
5. 对每个 owner 下载 `https://github.com/<owner>.png?size=96`，存到 `src/assets/library-logos/owners/<owner>.png`。
6. 在 `NOTICE.md` 里追加一节，说明数据来源（magpie，MIT）和头像来源（GitHub 头像）。

Run: `node scripts/build-popular-skills.mjs && node -e 'const j=require("./src/components/library/popularSkills.json"); console.log(j.skills.length)'`
Expected: 输出不超过 60 的数字；`owners/` 下的文件都是 PNG。

- [ ] **Step 2: 写失败的测试**

```ts
it("formats install counts", () => {
  expect([0, 999, 1000, 954000, 1234567].map(formatInstalls))
    .toEqual(["0", "999", "1K", "954K", "1.2M"]);
});
it("keeps at most four skills per repository and at most 60", () => {
  expect(POPULAR_SKILLS.length).toBeLessThanOrEqual(60);
  const per = new Map<string, number>();
  for (const s of POPULAR_SKILLS) per.set(s.source, (per.get(s.source) ?? 0) + 1);
  expect(Math.max(...per.values())).toBeLessThanOrEqual(4);
});
it("sorts by installs, highest first", () => {
  const n = POPULAR_SKILLS.map((s) => s.installs);
  expect([...n].sort((a, b) => b - a)).toEqual(n);
});
```

在 `LibrarySkillDiscover.test.tsx` 里加：
- `shows popular skills before any search`：渲染发现页、不输入任何内容，能看到第一个热门技能的名称、安装量（短格式）和「添加」按钮。
- `shows no undefined text for a skill without a description`：在测试里注入一条没有描述的技能，卡片上不出现「undefined」。
- `adds a popular skill through the existing skills.sh install path`：点「添加」后，调用的是现有 skills.sh 安装用的那个 API，参数里带有 `source` 和 `skillId`。具体是哪个函数，看现有的测试怎么 mock。

- [ ] **Step 3: 运行，确认失败**

Run: `pnpm exec vitest run tests/components/library/popularSkills.test.ts tests/components/library/LibrarySkillDiscover.test.tsx`
Expected: FAIL。

- [ ] **Step 4: 实现**

- 发现页默认显示「热门」：卡片网格，宽窗口 4 列，窄一些时 3 列或 2 列，参考 magpie 的样式。
- 每张卡片有：作者头像（用 Task 1 的组件，`name` 传 owner）、名称、官方 ✓、来源仓库、`↓ 1.2M`、一行描述（最多两行，超出省略）和「添加」按钮。
- 搜索结果的行也显示安装量和图标。
- 输入框为空时显示热门，有内容时显示搜索结果。原有的分段控件（仓库 / skills.sh / 精选）保留，「热门」作为默认视图。

- [ ] **Step 5: 运行，确认通过**

Run: `pnpm exec vitest run tests/components/library/`
Expected: 全部 PASS。

- [ ] **Step 6: 提交**

```bash
git add scripts/build-popular-skills.mjs src/components/library src/assets/library-logos src/i18n tests/components/library
git commit -m "feat(library): popular skills from a bundled skills.sh snapshot with installs and avatars"
```

---

### Task 5: 浏览器里看效果，然后整体验收

- [ ] **Step 1: 在浏览器里加载真实样式检查**

用模拟 IPC 的测试页在 Chrome 里打开资源库：
- 可以从 `/Users/yunshen/code/ofox-desktop-assistant-media/.harness/` 复制 `console.tsx` 和 `vite.config.mjs` 到本工作区的 `.harness/`（这个目录已经被 git 忽略）。
- 启动：`pnpm exec vite --config .harness/vite.config.mjs --port 4531`，打开 `?view=console`，切到「资源库」页。

要看的：
- 窄窗口 1024px 和宽窗口 1512px，亮色和暗色，指令、MCP、技能三个子页都看一遍。
- 每张卡片都有图标，没有破图。
- 「需登录」「需要 API key」标记清楚可见。
- 热门技能的卡片网格不溢出。

截图存到本工作区的 `.harness-shots/`。

- [ ] **Step 2: 整体检查**

Run: `pnpm typecheck && pnpm format:check && pnpm test:unit 2>&1 | tail -4 && cargo fmt --check --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -2 && cargo test --manifest-path src-tauri/Cargo.toml library 2>&1 | grep "test result" | tail -3`
Expected: 全部通过，clippy 没有警告。

- [ ] **Step 3: 收尾**

- 如果 Step 1 发现问题，修好后单独提交，提交信息写清楚修了什么。
- 最后在报告里写：
  - 哪些 MCP 没加，原因是什么；
  - 截图在哪里；
  - 测试数量。
