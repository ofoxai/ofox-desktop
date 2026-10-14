import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = path.resolve(__dirname, "../..");
const SRC = path.join(ROOT, "src");

// What the main window draws: everything MainApp reaches, plus the global
// stylesheet and the toaster main.tsx mounts beside it.
const ENTRIES = [
  "src/MainApp.tsx",
  "src/index.css",
  "src/components/ui/sonner.tsx",
];

// Tool identity, not UI chrome: the fallback badge colour of a tool with no
// logo of its own (Gemini and DeepSeek Harness both have logos).
const ALLOWLIST = new Set(["src/config/toolMeta.ts"]);

const BLUE =
  /\b(?:blue|sky|indigo)-\d{2,3}\b|#0a84ff|#409cff|59,\s*130,\s*246/gi;

function relative(file: string): string {
  return path.relative(ROOT, file).split(path.sep).join("/");
}

function resolveImport(from: string, spec: string): string | null {
  let base: string;
  if (spec.startsWith("@/")) base = path.join(SRC, spec.slice(2));
  else if (spec.startsWith(".")) base = path.resolve(path.dirname(from), spec);
  else return null;
  for (const ext of ["", ".tsx", ".ts", "/index.tsx", "/index.ts"]) {
    const candidate = base + ext;
    if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) {
      return candidate;
    }
  }
  return null;
}

function mainWindowFiles(): string[] {
  const seen = new Set<string>();
  const queue = ENTRIES.map((entry) => path.join(ROOT, entry));
  while (queue.length) {
    const file = queue.pop()!;
    if (seen.has(file) || !/\.(tsx?|css)$/.test(file)) continue;
    seen.add(file);
    const text = fs.readFileSync(file, "utf8");
    for (const m of text.matchAll(
      /(?:from|import)\s*\(?\s*["']([^"']+)["']/g,
    )) {
      const next = resolveImport(file, m[1]);
      if (next) queue.push(next);
    }
  }
  return [...seen];
}

describe("no blue in the main window", () => {
  it("walks the main window's own files", () => {
    const files = mainWindowFiles().map(relative);
    expect(files).toContain("src/components/console/ConsolePage.tsx");
    expect(files).toContain("src/components/ui/button.tsx");
  });

  it("draws nothing reachable from the main window in blue", () => {
    const offenders = mainWindowFiles()
      .filter((file) => !ALLOWLIST.has(relative(file)))
      .flatMap((file) => {
        const text = fs.readFileSync(file, "utf8");
        return [...text.matchAll(BLUE)].map((m) => {
          const line = text.slice(0, m.index).split("\n").length;
          return `${relative(file)}:${line} ${m[0]}`;
        });
      });
    expect(offenders, `blue found at:\n${offenders.join("\n")}`).toEqual([]);
  });
});
