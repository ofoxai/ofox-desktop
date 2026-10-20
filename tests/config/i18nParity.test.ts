import { describe, expect, it } from "vitest";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";

type Tree = { [key: string]: unknown };

const LOCALES: Record<string, Tree> = { zh, en, ja };

function flatten(tree: Tree, prefix = "", out = new Map<string, unknown>()) {
  for (const [key, value] of Object.entries(tree)) {
    const path = prefix ? `${prefix}.${key}` : key;
    if (value && typeof value === "object" && !Array.isArray(value)) {
      flatten(value as Tree, path, out);
    } else {
      out.set(path, value);
    }
  }
  return out;
}

function placeholders(text: string): string[] {
  const names = [...text.matchAll(/\{\{\s*([^}\s]+)\s*\}\}/g)].map((m) => m[1]);
  return [...new Set(names)].sort();
}

const flat = Object.fromEntries(
  Object.entries(LOCALES).map(([lang, tree]) => [lang, flatten(tree)]),
);
const allKeys = [
  ...new Set(Object.values(flat).flatMap((map) => [...map.keys()])),
].sort();

describe("i18n parity across zh / en / ja", () => {
  it("every locale has exactly the same keys", () => {
    const missing = Object.entries(flat).flatMap(([lang, map]) =>
      allKeys.filter((key) => !map.has(key)).map((key) => `${lang}: ${key}`),
    );
    expect(missing).toEqual([]);
  });

  it("every value is a non-empty string", () => {
    const bad = Object.entries(flat).flatMap(([lang, map]) =>
      [...map.entries()]
        .filter(([, value]) => typeof value !== "string" || !value.trim())
        .map(([key]) => `${lang}: ${key}`),
    );
    expect(bad).toEqual([]);
  });

  it("every key uses the same {{placeholders}} in each locale", () => {
    const mismatched = allKeys.flatMap((key) => {
      const sets = Object.entries(flat).map(([lang, map]) => {
        const value = map.get(key);
        return [lang, typeof value === "string" ? placeholders(value) : []];
      });
      const distinct = new Set(sets.map(([, names]) => String(names)));
      return distinct.size > 1 ? [`${key}: ${JSON.stringify(sets)}`] : [];
    });
    expect(mismatched).toEqual([]);
  });
});
