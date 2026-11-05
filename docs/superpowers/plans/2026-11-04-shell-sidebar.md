# Sidebar, Page Titles and Loading Fox Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the top bar / 80px rail with a grouped, collapsible, Ofox-branded left sidebar (account + balance on top, sub-pages under the open page), give every page a title row with breathing room, put the assistant in three frameless columns at every width, and show a cute animated fox while a page first loads.

**Architecture:** The shell keeps owning page selection; pages keep owning their sub-page state and register a `ShellSubnav` (items, value, onChange) that the sidebar renders, so leave guards, scroll memory and location requests stay inside the pages. A shared `PageHeader` (sticky, drags the window on macOS) carries title, description and actions. `FoxLoader` is a pure SVG/CSS component with a delay/minimum-display hook.

**Tech Stack:** React 18 + TypeScript, Tailwind (tokens in `src/index.css`), lucide-react, Radix popover/dropdown/tooltip from `src/components/ui`, i18next (zh/en/ja), Vitest + React Testing Library.

**Spec:** `docs/superpowers/specs/2026-11-04-shell-sidebar-design.md` (mockups: `.superpowers/brainstorm/18440-1793792468/content/`, git-excluded).

## Global Constraints

- All visible text through i18n in `src/i18n/locales/{zh,en,ja}.json`; Chinese copy below is the reference, write en/ja to match.
- Colours only from tokens (`canvas`, `card`, `line`, `line-2`, `ink*`, `brand`, `brand-soft`, `brand-ink`, `danger`); check light and dark.
- Active page highlight: `bg-brand-soft text-brand-ink font-semibold`, icon `text-brand`. Active sub-page: `text-brand-ink font-semibold`. No white pill, no "a / b / c" breadcrumb.
- Region URLs only via `src/lib/ofoxUrls.ts` + `useOfoxApex` (top-up = `ofoxWalletUrl(apex)`).
- macOS: traffic lights stay at (14,17) — no Rust change. Drag areas use `data-tauri-drag-region` only when `isMac()`; interactive children get `no-drag`.
- Pages stay mounted once visited; nothing in this plan may remount a page panel or reset a page's state.
- Storage keys: `ofox.shell.sidebarCollapsed` (new), `ofox.assistant.sidebarCollapsed`, `ofox.usageTab` (existing). Every localStorage access in try/catch.
- Each commit: `pnpm typecheck`, `pnpm format:check`, `pnpm test:unit` pass (pnpm = `node ~/.cache/node/corepack/v1/pnpm/10.34.5/bin/pnpm.cjs`). Conventional commit subject, body ends with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`. No push.
- Open source: draw the fox from Ofox's own logo shape; no magpie code or art.

## Review Focus

1. Switching a library sub-page from the sidebar while a library editor has unsaved changes must go through the library's leave guard (same as the old in-page tab) — test in Task 5.
2. `shell.locateLibrary("skills")` / `shell.locateUsage(key)` from other pages must still land on the right sub-page and the sidebar must highlight it — tests in Tasks 5 and 6.
3. Usage sub-pages that the data source does not support (`requestSupported` / `analysisSupported` false) must not appear in the sidebar or the collapsed flyout — test in Task 6.
4. Collapsed sidebar by keyboard only: Tab to an icon opens its flyout, ArrowDown reaches a sub-page, Enter switches, Escape closes and returns focus to the icon — test in Task 3.
5. A spent balance (≤ 0) shows in `text-danger` in the account block, and a failed user load offers retry there (the tools page no longer has the card) — tests in Task 3.

---

### Task 1: FoxLoader

**Files:**
- Create: `src/components/ui/fox-loader.tsx`, `src/hooks/useDelayedVisible.ts`
- Modify: `src/index.css` (keyframes `fox-hop`, `fox-shadow`, `fox-tilt`, `fox-ear-l`, `fox-ear-r`, `fox-blink`, `fox-bar`, all inside `@media (prefers-reduced-motion: no-preference)`)
- Test: `tests/components/FoxLoader.test.tsx`, `tests/hooks/useDelayedVisible.test.ts`

**Interfaces:**
- Produces: `FoxLoader({ size?: "page" | "block"; label?: string; className?: string }): JSX.Element | null` — `role="status"`, `aria-live="polite"`, accessible name = `label ?? t("common.loadingFox")`. `page`: 60px fox, shadow, 110px brand progress bar, label under it, centred with `py-16`. `block`: 36px fox + shadow, label optional (shown only when passed), `py-8`. Returns `null` until `useDelayedVisible(true)` says visible.
- Produces: `useDelayedVisible(active: boolean, delayMs = 200, minMs = 600): boolean` — becomes true only after `active` stayed true for `delayMs`; once true, stays true at least `minMs` even if `active` turns false.
- Produces: i18n `common.loadingFox` = 「加载中…」.

- [ ] **Step 1: Write failing hook tests** (fake timers): `stays hidden for a load shorter than 200ms` (active true→false at 150ms ⇒ never true); `shows after 200ms`; `once shown stays at least 600ms` (shown at 200, active false at 300 ⇒ still true at 700, false at 801).
- [ ] **Step 2: Write failing component tests:** `renders nothing before the delay`; after `vi.advanceTimersByTime(200)`: `getByRole("status", { name: "Loading…" })`; `page size shows the progress bar` (`data-testid="fox-bar"`), `block size has no bar`; svg has `data-part="ear-l"`, `"ear-r"`, two `data-part="eye"`.
- [ ] **Step 3: Run** `pnpm test:unit tests/components/FoxLoader.test.tsx tests/hooks/useDelayedVisible.test.ts` — FAIL (module not found).
- [ ] **Step 4: Implement.** SVG `viewBox="0 0 512 512"`, shapes copied from the approved mockup `loader-cute.html` symbol `#cute` (ears with inner `#E9A57A`, body `#C0703C` stroke 30 round joins, muzzle `#F6D9C4`, eyes `#2B1A10` with white highlight, blush `#F28B82` .55, nose + smile). Animation timing: one 2.8s cycle — hop with squash 0–32%, head tilt −12° 36–76%, left ear flick 46–62%, right ear flick 80–96%, blink at 73%; shadow scales with the hop. Under reduced motion nothing animates (keyframes only declared in the no-preference media block).
- [ ] **Step 5: Run tests** — PASS. `pnpm typecheck`.
- [ ] **Step 6: Commit** `feat(ui): a cute fox for first loads`.

### Task 2: Shell state — groups, collapsed sidebar, sub-page registry

**Files:**
- Modify: `src/components/shell/ShellContext.tsx`, `src/components/shell/useShellLayout.ts` (doc comment: now only "roomy sizes", see Task 7)
- Test: `tests/components/shell/ShellContext.test.tsx`

**Interfaces:**
- Produces in `ShellContext.tsx`:
  ```ts
  export const SHELL_GROUPS: ReadonlyArray<{ id: "use" | "observe"; pages: readonly ShellPage[] }> =
    [{ id: "use", pages: ["tools", "sessions", "library", "assistant"] },
     { id: "observe", pages: ["usage"] }];
  export const SHELL_PAGES: readonly ShellPage[] = SHELL_GROUPS.flatMap((g) => g.pages);
  export interface ShellSubnavItem<T extends string = string> { value: T; label: string; count?: number; controls: string }
  export interface ShellSubnav<T extends string = string> { items: ReadonlyArray<ShellSubnavItem<T>>; value: T; onChange: (value: T) => void }
  // ShellContextValue gains:
  subnavs: Partial<Record<ShellPage, ShellSubnav>>;
  sidebarCollapsed: boolean;
  setSidebarCollapsed: (collapsed: boolean) => void;
  export function useShellSubnav<T extends string>(page: ShellPage, nav: ShellSubnav<T> | null): void;
  ```
- `useShellSubnav` publishes on mount/update, withdraws on unmount; `onChange` always calls the latest callback (ref), so callers need not memoise it; republish only when `value` or the items' `value/label/count/controls` change (no render loop).
- `sidebarCollapsed` initial value read from `localStorage["ofox.shell.sidebarCollapsed"] === "1"`, default false; writes "1"/"0"; storage errors ignored.
- i18n: `shell.groups.use` 「使用」, `shell.groups.observe` 「观测」, `shell.sidebar.collapse` 「收起侧栏」, `shell.sidebar.expand` 「展开侧栏」.

- [ ] **Step 1: Failing tests:** `SHELL_PAGES follows the groups` (equals `["tools","sessions","library","assistant","usage"]`); `a page's subnav is visible to the shell and withdrawn on unmount`; `subnav onChange calls the page's latest callback`; `collapsed state survives a remount via localStorage`; `collapsed state works when localStorage throws` (spy `Storage.prototype.getItem` to throw ⇒ false, toggling still works).
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement** in `ShellContext.tsx`.
- [ ] **Step 4: Run** this test plus `tests/components/shell/ShellLayout.test.tsx` — PASS.
- [ ] **Step 5: Commit** `feat(shell): page groups, a remembered sidebar state and sub-pages pages can publish`.

### Task 3: AppSidebar replaces the top bar and the rail

**Files:**
- Create: `src/components/shell/AppSidebar.tsx`, `src/components/shell/SidebarAccount.tsx`
- Modify: `src/components/shell/ShellFrame.tsx`, `src/components/shell/UpdatePill.tsx` (add `variant: "row" | "icon"`; remove the header form), `src/components/console/ConsolePage.tsx` (lift `useOfoxAccount` into `ConsoleShell` for the signed mode, pass it to `ToolsPanel`, `UsagePage` and the sidebar)
- Delete: `src/components/shell/AppHeader.tsx`, `src/components/shell/AppRail.tsx`, `RailNav` in `src/components/ui/segmented.tsx` if unused, `tests/components/shell/AppHeader.test.tsx`, `tests/components/shell/AppRail.test.tsx`
- Test: `tests/components/shell/AppSidebar.test.tsx`, `tests/components/shell/SidebarAccount.test.tsx`; update `tests/components/ConsolePage.test.tsx`, `tests/components/ConsoleSettingsFocus.test.tsx` where they query the old header

**Interfaces:**
- Consumes: Task 2 (`SHELL_GROUPS`, `subnavs`, `sidebarCollapsed`, `setSidebarCollapsed`).
- Produces:
  ```ts
  export type SidebarAccountState =
    | { kind: "signed"; account: OfoxAccount }
    | { kind: "local"; signedIn: boolean; onAction: () => void };
  export function AppSidebar(props: { account: SidebarAccountState; settingsButtonRef?: Ref<HTMLButtonElement>; navRef?: Ref<HTMLElement> }): JSX.Element;
  export function SidebarAccount(props: { state: SidebarAccountState; collapsed: boolean }): JSX.Element;
  export function ShellFrame(props: { account: SidebarAccountState; settingsButtonRef?: Ref<HTMLButtonElement>; banner: ReactNode; children: ReactNode }): JSX.Element;
  ```
- Layout (expanded, `w-[216px]`, `bg-canvas`, right border `line-2`, `px-2.5 pb-3`):
  1. Top row `h-[52px]`, drag region on mac; on mac the left 78px is left empty for the lights; collapse toggle (`PanelLeft` icon, `variant="icon"`, aria-label `shell.sidebar.collapse`) at the right. Non-mac: row `h-3` and the toggle moves into the account row's right edge.
  2. `SidebarAccount`: card (`bg-card border-line rounded-[10px] px-2.5 py-2`), `UserAvatar` 28px (fallback: `bg-brand-soft` circle with the fox logo), name (12.5px semibold, truncate), line 2 「余额 $923.80」 in `text-brand-ink`, `text-danger` when balance ≤ 0, 「余额 —」 while unknown. Whole card is a `DropdownMenu` trigger: items 「充值」 (only when `isOfoxBillingManager(user)`, opens `ofoxWalletUrl(apex)` via `settingsApi.openExternal`), 「刷新余额」 (`account.refreshBalance()`), 「账户设置」 (`shell.openSettings()`). `account.retrying` ⇒ line 2 shows spinner + `console.balance.loadingUser[Attempt]`; `account.loadFailed` ⇒ line 2 is a button 「重新加载」 calling `account.retryUser()` (outside the menu trigger). Local: `signedIn=false` ⇒ brand button 「登录 Ofox」 (`toolCatalog.login`), `signedIn=true` ⇒ brand button `toolCatalog.bindOptional`; both call `onAction`.
  3. Groups from `SHELL_GROUPS`: group label `text-[11px] text-ink-muted px-2.5 pt-3.5 pb-1.5`; page item `h-8 px-2.5 gap-2.5 rounded-[8px] text-[13.5px] text-ink-2`, lucide icon 16px (tools `Wrench`, sessions `MessagesSquare`, library `LibraryBig`, assistant `Sparkles`, usage `ChartColumn`). Items are `role="tab"` in one `role="tablist" aria-orientation="vertical"` with `aria-controls={shellPanelId(page)}`, roving tabindex, ArrowUp/Down move, Home/End.
  4. Under the active page only, its `subnavs[page]` items: indented `ml-[22px]`, `h-7 px-2.5 text-[13px]`, count right-aligned `text-[11px] text-ink-muted`, in their own `role="tablist"` with `aria-controls={item.controls}`.
  5. Foot (`mt-auto`): `UpdatePill variant="row"` (full-width brand-soft row 「新版本 v{{version}}」, same menu as today), then 「设置」 item (`Settings` icon) with `settingsButtonRef`.
- Collapsed (`w-20`): same top row (toggle shows `shell.sidebar.expand`), account = avatar only (menu still works; local = icon button with the same action and a tooltip), page icons only `h-9 w-9` centred with tooltip = page name, foot = update icon + settings icon. Each page icon has a flyout (Radix `Popover`, `side="right"`, opens on pointer enter and on keyboard focus, closes on pointer leave after 120ms or Escape): first line the page name, then its subnav items; clicking the icon itself still `setActive(page)`.
- `ShellFrame` always renders `AppSidebar` beside `<div class="flex min-w-0 flex-1 flex-col">{banner}<main/></div>`; remove the narrow/wide navigation switch and the focus hand-over code.

- [ ] **Step 1: Failing `AppSidebar` tests:** `shows the two groups with pages in order`; `clicking a page selects its panel` (`aria-selected`, panel not hidden); `active page uses the brand highlight` (class contains `bg-brand-soft`); `shows only the active page's sub-pages and switches them` (register a fake subnav via `useShellSubnav` in a test page); `collapse hides labels, remembers it, and expand restores`; `collapsed: focusing an icon opens its flyout, ArrowDown + Enter switches the sub-page, Escape closes and focus returns to the icon` (Review Focus 4); `settings opens through openSettings`; `update row shows when an update exists`; `mac leaves room for the traffic lights and drags from the top row` (`data-tauri-drag-region` present when `isMac()` true, absent when false).
- [ ] **Step 2: Failing `SidebarAccount` tests:** `signed: name and balance`; `balance ≤ 0 is shown in danger` (Review Focus 5); `load failure offers a retry that calls retryUser` (Review Focus 5); `menu: top-up only for a billing manager and opens the wallet URL of the current apex`; `menu: refresh balance and account settings`; `local signed out shows 登录 Ofox`; `local signed in shows bind tools`; `collapsed shows only the avatar`.
- [ ] **Step 3: Run** — FAIL.
- [ ] **Step 4: Implement** the two components, `ShellFrame`, `UpdatePill` variants, the `ConsolePage` account lift (one `useOfoxAccount()` instance in `ConsoleShell` when not `localOnly`; `SignedToolsPanel`/`SignedUsagePage` receive it instead of calling the hook). Delete header/rail files and their tests; fix remaining imports.
- [ ] **Step 5: Run** `pnpm test:unit` (whole suite) and `pnpm typecheck` — PASS.
- [ ] **Step 6: Commit** `feat(shell): a grouped Ofox sidebar with the account on top`.

### Task 4: PageHeader, tools and sessions pages

**Files:**
- Create: `src/components/shell/PageHeader.tsx`
- Modify: `src/components/console/ConsolePage.tsx` (panel wrappers: drop `pt-1`, padding moves into `PageHeader`/page body), `src/components/console/ToolsPanel.tsx` (remove `BalanceCard`; 方案 + 添加 into the header actions), `LocalToolsPanel` in `ConsolePage.tsx`, `src/components/sessions/SessionsPage.tsx` (header on top of its flex column)
- Delete: `src/components/console/BalanceCard.tsx` and `tests/components/BalanceCard.test.tsx` once unused (its behaviours are covered by Task 3's account tests)
- Test: `tests/components/shell/PageHeader.test.tsx`; update `tests/components/ConsolePage.test.tsx` / tools tests that looked for `balance-card`

**Interfaces:**
- Produces:
  ```ts
  export function PageHeader(props: { title: string; description?: string; actions?: ReactNode; className?: string }): JSX.Element;
  export function ShellRefreshButton(): JSX.Element; // calls shell.refresh(), aria-busy + spin while shell.refreshing, disabled when !shell.canRefresh, label shell.refresh
  export const PAGE_BODY = "mx-auto w-full max-w-[1120px] px-7"; // page content container
  ```
- `PageHeader`: `sticky top-0 z-20 bg-canvas`, inner uses `PAGE_BODY`, `pt-9 pb-4` (title top ≈ 36px, level with the sidebar's account card), `<h1>` `text-[20px] font-semibold tracking-[-0.01em] text-ink`, description `mt-1 text-[12px] text-ink-muted`, actions `ml-auto flex items-center gap-2` aligned to the title line. On mac the header and its text drag the window (`data-tauri-drag-region`), actions get `no-drag`.
- Copy (zh): tools 「工具」/「用一个 Ofox 账户管理你的 AI 编程工具」; sessions 「会话」/「各工具的历史会话，搜索、续聊、查看用量」; local tools keeps `toolCatalog.localTitle` / `toolCatalog.localDescription` as title/description with its brand button as the action. Keys under `pages.<page>.title|description`.
- Tools actions: `<ProfilesMenu/>`, 添加 button, `<ShellRefreshButton/>`; the 「已绑定的工具 3」 caption and attention pill stay above the list. Sessions actions: `<ShellRefreshButton/>`.

- [ ] **Step 1: Failing tests:** `PageHeader renders an h1, description and actions`; `drags on mac, not elsewhere`; `ShellRefreshButton runs the open page's refresh and shows busy`; tools page: `no balance card; 方案 and 添加 sit in the page header` (`within(getByRole("banner"…))` or a `data-page-header` attribute); sessions: `has a 会话 heading`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `pnpm test:unit`, `pnpm typecheck` — PASS.
- [ ] **Step 5: Commit** `feat(shell): page titles with room to breathe on the tools and sessions pages`.

### Task 5: Library sub-pages in the sidebar

**Files:**
- Modify: `src/components/library/LibraryPage.tsx` (remove the in-page `SegmentedNav`; sticky header becomes `PageHeader`; publish subnav)
- Test: `tests/components/LibraryPage.test.tsx` (+ any `tests/components/library/*` that click the old tabs)

**Interfaces:**
- Consumes: `useShellSubnav` (Task 2), `PageHeader`, `ShellRefreshButton` (Task 4).
- `useShellSubnav("library", { items: KINDS.map(kind => ({ value: kind, label: t(`library.tabs.${kind}`), count: summary.data?.[kind], controls: `library-panel-${kind}` })), value: tab, onChange: (kind) => guarded(() => changeTab(kind)) })`.
- Title per sub-page (zh): 指令 「指令」/「给各工具共用的提示和规则」; mcp 「MCP 服务器」/「一处配置，分发到各工具」; skills 「技能」/「技能文件夹，按需链接到各工具」 (keys `pages.library.<kind>.title|description`). Actions: the existing scenario wizard, discover/templates button, `ShellRefreshButton`.
- `--library-scroll-offset` keeps measuring the (new) header element.

- [ ] **Step 1: Failing tests:** `sub-pages are published to the shell with counts`; `switching from the sidebar goes through an unsaved-changes guard` (Review Focus 1: register a guard that defers, assert the tab does not change until proceed); `locateLibrary("skills") lands on 技能 and the published value is skills` (Review Focus 2); `header title follows the sub-page`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** library tests + `pnpm typecheck` — PASS.
- [ ] **Step 5: Commit** `feat(library): sub-pages in the sidebar, a titled header`.

### Task 6: Usage sub-pages, header and one filter row

**Files:**
- Modify: `src/components/usage-overview/UsagePage.tsx`, `src/components/usage-overview/UsageFilterToolbar.tsx` (tools become a `UsageFilterMenu` with tool badges; chips/reset stay)
- Possibly modify: `src/components/usage-overview/UsageFilterMenu.tsx` (`UsageFilterOption.icon?: ReactNode` if not present)
- Test: existing usage tests under `tests/components/usage*` / `tests/components/Usage*`

**Interfaces:**
- Consumes: Tasks 2 and 4.
- Publish `useShellSubnav("usage", { items: [overview, requests if requestSupported, sessions if analysisSupported] (labels `usageWorkbench.overview`, `usageWorkbench.requests`, `usageSessions.title`, controls `usage-overview-panel` / `usage-requests-panel` / `usage-sessions-panel`), value: effectiveTab, onChange: <the old SegmentedNav onChange body> })`. Remove the in-page `SegmentedNav`.
- `PageHeader` title/description by sub-page (zh): 「用量概览」/「各工具这段时间用了多少 token、花了多少钱」; 「用量明细」/「每一次请求的模型、token 和费用」; 「会话分析」/「按会话看用量和耗时」. Actions: the refresh-interval `MenuSelect` and the usage refresh button (moved as is).
- First body row: period `SegmentedControl`, tools menu (「全部工具」), models menu, projects menu, then the estimate pushed right (`ml-auto`). The date/timezone note line sits under the row. The custom-range form keeps opening below.

- [ ] **Step 1: Failing tests:** `publishes 概览 / 明细 / 会话分析 when supported`; `omits 明细 when requests are unsupported` (Review Focus 3); `locateUsage(key) still opens the session analysis and the published value follows` (Review Focus 2); `tool filter is a menu: choosing Codex filters by codex`; `refresh interval and refresh live in the page header`.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** usage tests + `pnpm typecheck` — PASS.
- [ ] **Step 5: Commit** `feat(usage): sub-pages in the sidebar, a titled header and one filter row`.

### Task 7: Assistant — three frameless columns at every width

**Files:**
- Modify: `src/components/assistant/AssistantPage.tsx`, `src/components/assistant/AssistantSidebar.tsx`, `src/components/console/ConsolePage.tsx` (assistant panel always `h-full`, no padding)
- Test: `tests/components/AssistantWideLayout.test.tsx` (rename cases: frame vs sizes), `tests/components/AssistantConversations.test.tsx`, `tests/components/AssistantNavigation.test.tsx`

**Interfaces:**
- Split today's `wide` flag: `const roomy = shell?.layout === "wide"` keeps only the sizes (CSS variables `--fs-*`, `--icon-btn`, `--ctl-h`, the column `clamp(840px,72%,1040px)` vs `max-w-[760px]`, textarea min/max `[96,200]` vs `[76,160]`, reflow on change). Everything structural that was `wide`-only becomes unconditional: no rounded frame/border, list beside the transcript, top row drags on mac, title shown only when the list is collapsed or renaming, a new chat's composer centred.
- `AssistantSidebar` `wide` prop removed (always the frameless form).
- The `⋯` menu gains 「刷新」 (`shell.refresh` label `shell.refresh`) that calls `shell.refresh()`; disabled while `shell.refreshing`.

- [ ] **Step 1: Failing tests:** `at 1000px the assistant has no frame and the list sits beside the chat`; `sizes are roomy only from 1280px` (CSS variable present only when wide); `⋯ → 刷新 reloads the conversations` (the registered refresh handler runs); existing wide-layout tests keep passing for draft/scroll/picker preservation across the width switch.
- [ ] **Step 2: Run** — FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** assistant tests + `pnpm typecheck` — PASS.
- [ ] **Step 5: Commit** `feat(assistant): three frameless columns at every width, refresh in ⋯`.

### Task 8: The fox where pages first load

**Files:**
- Modify (replace first-load text/placeholder only, never background refresh): `src/components/console/ToolsPanel.tsx` (`console.tools.loading` card → `FoxLoader size="block" label={t("pages.tools.loading")}`), `src/components/usage-overview/UsagePage.tsx` + `UsageRequests.tsx` + `UsageSessions.tsx` (first load of each sub-page → `size="page"`, 「正在读取用量…」), `src/components/sessions/SessionsPage.tsx` / `SessionTranscript.tsx` (list first load `page` 「正在找你的会话…」; transcript first load `block`), `src/components/library/LibrarySection.tsx`, `LibraryMcpTable.tsx`, `LibrarySkillsPage.tsx` (first load `page` 「正在读取资源库…」), `src/components/assistant/AssistantPage.tsx` (opening a conversation whose messages are not loaded yet `block`).
- Test: one assertion per replaced spot in its existing test file: while the query is pending and 200ms passed, `getByRole("status", { name: <label> })`; while refetching with data present, no status fox.

- [ ] **Step 1:** grep each listed file for its loading branch; list the exact spots in the commit body.
- [ ] **Step 2: Failing tests** as above for tools, usage overview, sessions list, library skills, assistant conversation.
- [ ] **Step 3: Implement** (labels under `pages.<page>.loading`).
- [ ] **Step 4: Run** `pnpm test:unit`, `pnpm typecheck`, `pnpm format:check` — PASS.
- [ ] **Step 5: Commit** `feat(ui): the fox greets first loads of tools, usage, sessions, library and chats`.

### Task 9: Visual check and local build

**Files:** `.harness/` (git-excluded, existing Vite harness with mocked IPC) — extend its frame to render `ConsolePage` with fake account/usage data if it does not already.

- [ ] **Step 1:** Serve the harness; with chrome-devtools `emulate` take screenshots at 900, 1280, 1512, 1920 px wide, light and dark, of tools, usage 概览, library 技能, assistant (list open and collapsed), sidebar collapsed with a flyout open, and a FoxLoader. Save under `/Users/yunshen/code/ofox-desktop/.harness-shots/sidebar/`.
- [ ] **Step 2:** Check: content never touches the window top or sidebar edge; the 52px top row is empty where the traffic lights go; no horizontal scroll at 900px; dark mode legible. Fix and re-shoot as needed (commit fixes with the task they belong to).
- [ ] **Step 3:** Full gates: `pnpm typecheck && pnpm format:check && pnpm test:unit`.
- [ ] **Step 4:** Build and install the local app per memory (`pnpm tauri build --bundles app` with the pnpm shim, back up `~/.ofox-desktop` DB, move the old app to `~/.Trash`, move the new one in, `mdfind` single copy, check `ofoxApiKeys` createdAt unchanged after launch). Tell the user to ⌘Q and reopen.
