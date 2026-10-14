import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const ROOT = path.resolve(__dirname, "../..");
const SRC = path.join(ROOT, "src");

// Screens that still draw an OS-native <select>. Each entry is removed by the
// task that gives it the app's own menu; nothing may be added here.
const ALLOWLIST = new Set<string>([]);

function relative(file: string): string {
  return path.relative(ROOT, file).split(path.sep).join("/");
}

function tsxFiles(dir: string): string[] {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) return tsxFiles(full);
    return entry.name.endsWith(".tsx") ? [full] : [];
  });
}

function nativeSelects(file: string): string[] {
  const text = fs.readFileSync(file, "utf8");
  return [...text.matchAll(/<select[\s>]/g)].map((match) => {
    const line = text.slice(0, match.index).split("\n").length;
    return `${relative(file)}:${line}`;
  });
}

describe("no native <select>", () => {
  it("every dropdown in src is the app's own menu, not the OS popup", () => {
    const offenders = tsxFiles(SRC)
      .filter((file) => !ALLOWLIST.has(relative(file)))
      .flatMap(nativeSelects);
    expect(
      offenders,
      `native <select> found at:\n${offenders.join("\n")}`,
    ).toEqual([]);
  });

  it("the allowlist names only files that still need it", () => {
    const stale = [...ALLOWLIST].filter((rel) => {
      const file = path.join(ROOT, rel);
      return !fs.existsSync(file) || nativeSelects(file).length === 0;
    });
    expect(stale).toEqual([]);
  });
});
