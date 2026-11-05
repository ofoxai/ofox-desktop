# Wide Layout (left navigation, roomier assistant) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** When the window is wide, page navigation moves from the top bar to an 80px icon rail on the left, and the assistant page drops its frame and grows its type, controls and message column. Narrow windows stay exactly as they are.

**Architecture:** The shell measures the window and exposes `layout: "narrow" | "wide"` through `ShellContext`. `ConsolePage` keeps one stable tree: the top bar and the rail swap at the same position, so every page keeps its state. Size tokens become CSS variables whose fallbacks are today's values. The wide assistant page sets larger values on its own subtree.

**Tech Stack:** React 18, TypeScript, Tailwind 3.4, Vitest + Testing Library, Tauri 2 (macOS overlay title bar).

**Spec:** `docs/superpowers/specs/2026-11-04-assistant-wide-layout-design.md`

## Global Constraints

- Narrow layout (window under the threshold) must render exactly as before: same DOM classes, same computed sizes.
- Wide from window width ≥ 1280px. Back to narrow below 1240px. In between, the layout stays as it was.
- Rail width 80px, rail labels 10.5px. A label too long for the rail is truncated, with the full name in `title`.
- macOS: the traffic lights stay where they are (x=14, y=17). The rail's top 40px is a drag region holding nothing else. No Rust change.
- Wide assistant sizes:
  - Reply text 15px (from 13), notes 12px (from 11).
  - Icon buttons 30px (from 26). Menu triggers 30px tall (from 26).
  - Message column and composer: `clamp(840px, 72%, 1040px)`, centred.
- All visible text goes through i18n (zh, en, ja). No new settings.
- Each commit passes `pnpm typecheck`, `pnpm format:check` and `pnpm test:unit`.

## Review Focus

- Resizing across the threshold while a reply streams, or while a draft is typed: the draft, the stream and the scroll position survive. The test lives in Task 2.
- Resizing across the threshold while a menu or dialog is open: it closes or stays usable, and the page doesn't crash. Checked in the harness, Task 5.
- Window exactly at the threshold, or flickering between 1239 and 1281: no oscillation, thanks to the hysteresis. The test lives in Task 1.
- Keyboard focus on a top-bar tab when the layout switches: focus lands on the same page's rail tab, not on `body`. The test lives in Task 2.
- The list is collapsed in the wide layout and the conversation has no title yet: the top row shows the "New conversation" fallback. The test lives in Task 4.

---

### Task 1: The shell knows its layout

**Files:**
- Create: `src/components/shell/useShellLayout.ts`
- Modify: `src/components/shell/ShellContext.tsx` (add `layout` to `ShellContextValue`, provided from the hook)
- Test: `tests/components/shell/ShellLayout.test.tsx`

**Interfaces:**
- Produces: `export type ShellLayout = "narrow" | "wide"`, `export const WIDE_FROM = 1280`, `export const NARROW_BELOW = 1240`, `export function useShellLayout(): ShellLayout`, and `useShell().layout: ShellLayout`.

- [ ] **Step 1: Write the failing tests.** `it("is wide from 1280px and narrow again below 1240px")` sets `window.innerWidth` to each value in turn and dispatches `resize`, expecting:

  | width | layout |
  | ----- | ------ |
  | 1000  | narrow |
  | 1279  | narrow |
  | 1280  | wide   |
  | 1250  | wide   |
  | 1239  | narrow |
  | 1260  | narrow |

  `it("reads the width it opens at")` starts at 1512 and expects wide on the first render.
- [ ] **Step 2: Run** `pnpm exec vitest run tests/components/shell/ShellLayout.test.tsx`. Expect FAIL: module not found.
- [ ] **Step 3: Implement.** `useShellLayout` initialises from `window.innerWidth`, listens to `resize`, and applies the hysteresis from the previous value. `ShellProvider` calls it and puts it in the context value.
- [ ] **Step 4: Run** the test again. Expect PASS.
- [ ] **Step 5: Commit** `feat(shell): know whether the window is wide`.

### Task 2: The icon rail, and one tree for both layouts

**Files:**
- Modify: `src/components/ui/segmented.tsx` (add `RailNav`, which uses the existing `useRoving(…, true)`)
- Create: `src/components/shell/AppRail.tsx`
- Modify: `src/components/console/ConsolePage.tsx:145-151` (stable wrapper; header or rail at the first position)
- Modify: `src/i18n/locales/{zh,en,ja}.json` (only if the rail needs a new label; page labels reuse `shell.tabs.*`)
- Test: `tests/components/shell/AppRail.test.tsx`

**Interfaces:**
- Consumes: `useShell().layout`, `SHELL_PAGES`, `shellPanelId`.
- Produces: `RailNav<T>(props: SegmentedNavProps<T> & { icons: Record<T, ReactNode> })`, which renders `role="tablist"` with `aria-orientation="vertical"` and one `role="tab"` per item, holding the icon and the label. Also `AppRail({ settingsButtonRef })`.

- [ ] **Step 1: Write the failing tests.**
  - `it("puts the page tabs in the rail when the window is wide")`: at width 1512, `getByRole("tablist", { name: "Pages" })` has `aria-orientation="vertical"`, there is no `banner` header, and each tab's `aria-controls` equals `shellPanelId(page)`.
  - `it("keeps the top bar when the window is narrow")`: at width 1000, the tablist is horizontal and sits inside the `banner`.
  - `it("keeps a page as it was when the layout switches")`: type into an input inside the tools panel, resize 1000 → 1512 → 1000, then expect the input still has its value and is the same DOM node.
  - `it("moves the focus from a top-bar tab to the same page's rail tab")`.
  - `it("arrows through the rail with up and down")`.
- [ ] **Step 2: Run** `pnpm exec vitest run tests/components/shell/AppRail.test.tsx`. Expect FAIL.
- [ ] **Step 3: Implement.**
  - The rail is 80px wide with `bg-canvas`.
  - On macOS a 40px top strip carries `data-tauri-drag-region`.
  - Next comes the logo, then the `RailNav` tabs, each a 20px icon above a 10.5px label:

    | page      | icon             |
    | --------- | ---------------- |
    | tools     | `Wrench`         |
    | sessions  | `MessagesSquare` |
    | library   | `LibraryBig`     |
    | usage     | `ChartColumn`    |
    | assistant | `Sparkles`       |

  - Pinned to the bottom: `UpdatePill` (compact), refresh, settings.
  - `ConsolePage` renders:

    ```
    <div className={wide ? "flex h-screen w-full bg-canvas" : "flex h-screen w-full flex-col bg-canvas"}>
      {wide ? <AppRail … /> : <AppHeader … />}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        banner
        <main …>…</main>
      </div>
    </div>
    ```

  - Focus handover: before the switch, the layout effect remembers whether a top-bar or rail tab had focus. After the switch, it focuses the counterpart tab of `shell.active`.
- [ ] **Step 4: Run** the new test and `tests/components/shell`, `tests/components/console`. Expect PASS.
- [ ] **Step 5: Commit** `feat(shell): page navigation in a left rail when the window is wide`.

### Task 3: Size tokens as variables

**Files:**
- Modify: `tailwind.config.cjs` (`fontSize`: each token becomes `var(--fs-<name>, <today's px>)`)
- Modify: `src/components/ui/button.tsx:45` (the `icon` variant: `h-[var(--icon-btn,26px)] w-[var(--icon-btn,26px)]`)
- Modify: `src/components/ui/menu-select.tsx:73` (`h-[var(--ctl-h,26px)]`)
- Test: `tests/components/ui/sizeTokens.test.tsx`

**Interfaces:**
- Produces: the CSS variables below, all unset by default.

  | variable       | today's value (fallback) |
  | -------------- | ------------------------ |
  | `--fs-ui`      | 13px                     |
  | `--fs-nav`     | 12.5px                   |
  | `--fs-ctl`     | 12px                     |
  | `--fs-sub`     | 11.5px                   |
  | `--fs-note`    | 11px                     |
  | `--fs-micro`   | 10.5px                   |
  | `--fs-title`   | 15px                     |
  | `--icon-btn`   | 26px                     |
  | `--ctl-h`      | 26px                     |

- [ ] **Step 1: Write the failing test.** `it("keeps today's sizes when no variable is set")` requires `tailwind.config.cjs` and expects `fontSize.ui === "var(--fs-ui, 13px)"`, and the same for each token's value.
- [ ] **Step 2: Run** the test. Expect FAIL.
- [ ] **Step 3: Implement.** Change the three files exactly as listed.
- [ ] **Step 4: Run** `pnpm test:unit` and `pnpm build:renderer`. Expect PASS and a successful build.
- [ ] **Step 5: Commit** `refactor(ui): size tokens as variables with today's values`.

### Task 4: The wide assistant page

**Files:**
- Modify: `src/components/assistant/AssistantPage.tsx:933-1080` (root frame, header), `:1286-1300` (empty state), `:1352`, `:1538`, `:1553` (column width)
- Modify: `src/components/assistant/AssistantSidebar.tsx:379` (wide: `w-[260px]`, `bg-canvas`, no right border)
- Modify: `src/components/console/ConsolePage.tsx` (assistant panel padding: wide `p-0`)
- Test: `tests/components/AssistantWideLayout.test.tsx`

**Interfaces:**
- Consumes: `useShell().layout`; the CSS variables from Task 3.
- Produces: `data-layout="wide" | "narrow"` on the assistant root. When wide, the root sets these variables:

  | variable     | wide value |
  | ------------ | ---------- |
  | `--fs-ui`    | 15px       |
  | `--fs-ctl`   | 13.5px     |
  | `--fs-nav`   | 14px       |
  | `--fs-sub`   | 12.5px     |
  | `--fs-note`  | 12px       |
  | `--fs-micro` | 11.5px     |
  | `--fs-title` | 17px       |
  | `--icon-btn` | 30px       |
  | `--ctl-h`    | 30px       |

- [ ] **Step 1: Write the failing tests.** Use the `AssistantVersions.test.tsx` fixtures and wrap the page in `ShellProvider` at width 1512.
  - `it("shows no title in the top row while the list is open")`: no `heading` level 1. The model picker is in the top row, before ⋯.
  - `it("shows the title in the top row once the list is collapsed")`: click "Hide conversations", then expect the `heading` "Rewrites".
  - `it("renames in the top row even while the list is open")`: open ⋯, choose Rename, expect the textbox "Conversation title" in the top row; save it with `updateSession`.
  - `it("falls back to the page name for an untitled conversation when collapsed")`: with an empty title, the heading is "Assistant".
  - `it("starts a new conversation with its composer in the middle")`: the empty state contains the `Message` textbox.
  - `it("is unchanged in a narrow window")`: at width 1000, the `heading` level 1 is visible and the root has `rounded-list border`.
- [ ] **Step 2: Run** `pnpm exec vitest run tests/components/AssistantWideLayout.test.tsx`. Expect FAIL.
- [ ] **Step 3: Implement.**
  - When wide, the root drops `rounded-list border border-line` and sets the variables above (arbitrary properties, e.g. `[--fs-ui:15px]`).
  - Header when wide: the toggle and model picker on the left, ⋯ on the right, `data-tauri-drag-region` on macOS. The title (or the `InlineRename` field) shows only while collapsed or renaming.
  - Message column and composer wrappers: when wide, `w-[clamp(840px,72%,1040px)] max-w-full` replaces `max-w-[760px]`.
  - Empty state when wide: render the composer inside the empty state, above the starters, rather than in the footer. Use the same composer element, moved and not duplicated.
- [ ] **Step 4: Run** the new test, then `pnpm test:unit`. Expect PASS. The existing assistant tests, which run at jsdom's default 1024px width, stay narrow and unchanged.
- [ ] **Step 5: Commit** `feat(assistant): a roomier page without its frame when the window is wide`.

### Task 5: Verify in a real browser, build, install

**Files:**
- Modify: `.harness/` (git-excluded)
  - Render `ShellProvider`, then `ConsolePage`'s frame with the assistant page.
  - Alias `@/lib/api/assistant` to a fake `assistantApi` seeded with a conversation that has text, an image and a video.

- [ ] **Step 1: Screenshot the narrow layout before and after.** Take screenshots at 1000×700 in light and dark, at the commit before Task 2 and at HEAD, then diff them. Expect them to be identical.
- [ ] **Step 2: Screenshot the wide layout.** Take screenshots at 1512×945 and 1920×1080, light and dark, with the list open and collapsed. Check the margin each side of the column is at most about 15% of the chat area, and nothing overlaps the 78px traffic-light corner.
- [ ] **Step 3: Resize while busy.** Resize across the threshold with a draft typed, with a menu open, and with a dialog open. Expect no lost draft and no console error.
- [ ] **Step 4: Run the full checks.** `pnpm typecheck`, `pnpm format:check`, `pnpm test:unit`. Run `cargo test` only if Rust changed (it should not).
- [ ] **Step 5: Install and record.** Do the signed build and install, tick Fizzy #1460 steps 04 and 05 and comment, and update the memory.
