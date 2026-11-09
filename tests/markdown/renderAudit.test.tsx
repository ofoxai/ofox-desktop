import { readdirSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { render, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { hasMathMarkers, loadMarkdownMath } from "@/lib/markdownMath";

// `RENDER_AUDIT_DIR=~/.ofox-desktop/assistant pnpm render:audit` draws every
// assistant reply saved there as the app does and lists the ones that render
// wrong. Skipped when the variable is unset.
const root = process.env.RENDER_AUDIT_DIR;

function sessionFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return sessionFiles(path);
    return name === "session.json" ? [path] : [];
  });
}

/** What went wrong drawing one reply, or nothing. */
function problems(container: HTMLElement): string[] {
  const copy = container.cloneNode(true) as HTMLElement;
  copy.querySelectorAll("code, pre, .katex").forEach((node) => node.remove());
  const text = copy.textContent ?? "";
  const found: string[] = [];
  for (const marker of ["**", "~~"]) {
    const at = text.indexOf(marker);
    if (at >= 0)
      found.push(
        `literal ${marker}: …${text.slice(Math.max(0, at - 12), at + 14)}…`,
      );
  }
  for (const error of container.querySelectorAll(".katex-error"))
    found.push(`formula not drawn: ${(error.textContent ?? "").slice(0, 60)}`);
  for (const link of container.querySelectorAll("a[href]")) {
    const href = link.getAttribute("href") ?? "";
    if (/[*~]|[　-〿！-／：-＠]/.test(decodeURI(href)))
      found.push(`link swallowed its tail: ${href.slice(0, 60)}`);
  }
  return found;
}

describe.skipIf(!root)("render audit", () => {
  it("draws every saved assistant reply without a rendering problem", async () => {
    await loadMarkdownMath();
    const findings: string[] = [];
    let replies = 0;
    for (const file of sessionFiles(root!)) {
      const saved = JSON.parse(readFileSync(file, "utf8")) as {
        session?: { title?: string };
        messages?: { role: string; content?: string }[];
      };
      for (const [index, entry] of (saved.messages ?? []).entries()) {
        if (entry.role !== "assistant" || !entry.content) continue;
        replies += 1;
        const { container, unmount } = render(
          <SessionMarkdown content={entry.content} math />,
        );
        if (hasMathMarkers(entry.content))
          await waitFor(() =>
            expect(
              container.querySelector(".katex, .katex-error"),
            ).not.toBeNull(),
          ).catch(() => undefined);
        for (const problem of problems(container))
          findings.push(
            `${saved.session?.title ?? file} #${index}: ${problem}`,
          );
        unmount();
      }
    }
    expect(replies).toBeGreaterThan(0);
    expect(findings).toEqual([]);
  }, 120_000);
});
