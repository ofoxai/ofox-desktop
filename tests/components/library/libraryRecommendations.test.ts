import { describe, expect, it } from "vitest";
import {
  FlaskConical,
  Bug,
  GitCommitHorizontal,
  LayoutTemplate,
  BookOpen,
  ShieldCheck,
  Shuffle,
  ListChecks,
  Zap,
} from "lucide-react";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import {
  TEMPLATE_ICONS,
  hasRecipeLogo,
} from "@/components/library/LibraryResourceIcon";
import {
  LIBRARY_RECOMMENDATIONS,
  cloneRecommendationDraft,
  createMcpDraft,
  needsApiKey,
  type LibraryRecommendation,
} from "@/components/library/libraryRecommendations";

const ids = [
  "exa",
  "memory",
  "sequential-thinking",
  "git",
  "time",
  "sentry",
  "linear",
  "notion",
  "atlassian",
  "supabase",
  "stripe",
  "vercel",
  "neon",
  "cloudflare-docs",
  "huggingface",
  "brave-search",
  "firecrawl",
  "tavily",
];
const mcp = LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "mcp");
function get(value: unknown, key: string): unknown {
  return key
    .split(".")
    .reduce<unknown>(
      (current, part) =>
        current && typeof current === "object"
          ? (current as Record<string, unknown>)[part]
          : undefined,
      value,
    );
}

describe("verified MCP recommendations", () => {
  it("ships all 18 verified additions alongside the original eight", () => {
    expect(mcp).toHaveLength(26);
    for (const id of ids) expect(mcp.some((r) => r.id === id)).toBe(true);
    expect(new Set(LIBRARY_RECOMMENDATIONS.map((r) => r.id)).size).toBe(
      LIBRARY_RECOMMENDATIONS.length,
    );
  });
  it("gives every MCP a bundled logo and translated display metadata", () => {
    for (const r of mcp) {
      expect(hasRecipeLogo(r.id), r.id).toBe(true);
      for (const locale of [zh, en, ja]) {
        for (const key of [
          r.descriptionKey,
          r.runtimeKey,
          r.preparationKey,
          `library.discover2.categories.${r.category}`,
          `library.discover2.licenses.${r.licenseKey}`,
          ...r.dependencyKeys,
          ...r.permissionKeys,
        ]) {
          expect(key, r.id).toBeTruthy();
          expect(get(locale, key!), `${r.id}: ${key}`).toBeTruthy();
        }
      }
    }
    expect(hasRecipeLogo("__proto__")).toBe(false);
    expect(hasRecipeLogo("https://example.com/logo.png")).toBe(false);
  });
  it("marks only the browser sign-in additions", () => {
    const expected = [
      "sentry",
      "linear",
      "notion",
      "atlassian",
      "supabase",
      "stripe",
      "vercel",
      "neon",
    ];
    for (const id of ids)
      expect(Boolean(mcp.find((r) => r.id === id)?.signIn), id).toBe(
        expected.includes(id),
      );
  });
  it("keeps upstream API key inputs empty with their exact names", () => {
    for (const [id, name] of [
      ["brave-search", "BRAVE_API_KEY"],
      ["firecrawl", "FIRECRAWL_API_KEY"],
      ["tavily", "TAVILY_API_KEY"],
    ]) {
      const r = mcp.find((r) => r.id === id)!;
      expect(needsApiKey(r)).toBe(true);
      expect(r.initialDraft.environment).toEqual([
        { id: expect.any(String), name, value: "" },
      ]);
      const draft = cloneRecommendationDraft(r);
      draft.environment[0].value = "private-key";
      expect(r.initialDraft.environment[0].value).toBe("");
    }
    expect(needsApiKey(mcp.find((r) => r.id === "fetch")!)).toBe(false);
  });
  it("detects empty environment or header values, but ignores supplied values and non-MCP resources", () => {
    const recipe = (
      initial: Parameters<typeof createMcpDraft>[0],
    ): LibraryRecommendation => ({
      ...mcp[0],
      kind: "mcp",
      initialDraft: createMcpDraft(initial),
    });
    expect(
      needsApiKey(
        recipe({ headers: [{ id: "h", name: "Authorization", value: "" }] }),
      ),
    ).toBe(true);
    expect(
      needsApiKey(
        recipe({
          environment: [{ id: "e", name: "KEY", value: "already set" }],
        }),
      ),
    ).toBe(false);
    expect(
      needsApiKey(
        LIBRARY_RECOMMENDATIONS.find((r) => r.kind === "instructions")!,
      ),
    ).toBe(false);
  });
  it("groups search services together and records the actual runtime", () => {
    for (const id of ["exa", "brave-search", "firecrawl", "tavily"])
      expect(mcp.find((r) => r.id === id)?.category).toBe("search");
    for (const id of ids) {
      const r = mcp.find((r) => r.id === id)!;
      expect(r.runtimeKey).toBe(
        `library.discover2.runtime.${r.initialDraft.transport === "http" ? "http" : r.initialDraft.command === "uvx" ? "uv" : "node"}`,
      );
      expect(r.toolIds).toEqual([
        "claude",
        "codex",
        "gemini",
        "opencode",
        "hermes",
      ]);
    }
  });
});

const templates = LIBRARY_RECOMMENDATIONS.filter(
  (r) => r.kind === "instructions",
);
const templateIds = [
  "coding",
  "chinese",
  "review",
  "tdd",
  "debugging",
  "git-commits",
  "frontend",
  "docs-writing",
  "security",
  "refactor",
  "plan-first",
  "concise",
];

describe("instruction templates", () => {
  it("ships 12 instruction templates", () => {
    expect(templates).toHaveLength(12);
    expect(templates.map((t) => t.id)).toEqual(templateIds);
    expect(
      LIBRARY_RECOMMENDATIONS.filter((r) => r.kind === "skills"),
    ).toHaveLength(6);
  });
  it("writes every template in three languages, 10 to 25 lines each", () => {
    for (const t of templates)
      for (const lng of ["zh", "en", "ja"] as const) {
        const lines = t.contentByLanguage[lng].trim().split("\n").length;
        expect(lines, `${t.id}: ${lng}`).toBeGreaterThanOrEqual(10);
        expect(lines, `${t.id}: ${lng}`).toBeLessThanOrEqual(25);
      }
  });
  it("gives every template its own icon", () => {
    expect(Object.keys(TEMPLATE_ICONS)).toHaveLength(12);
    for (const [id, icon] of Object.entries({
      tdd: FlaskConical,
      debugging: Bug,
      "git-commits": GitCommitHorizontal,
      frontend: LayoutTemplate,
      "docs-writing": BookOpen,
      security: ShieldCheck,
      refactor: Shuffle,
      "plan-first": ListChecks,
      concise: Zap,
    })) {
      expect(TEMPLATE_ICONS[id]?.icon, id).toBe(icon);
    }
    for (const id of templateIds) {
      const color = TEMPLATE_ICONS[id]?.className;
      expect(color, id).toMatch(
        /^bg-(red|orange|green|rose|violet|emerald|amber|cyan)-100 text-\1-800 dark:bg-\1-900 dark:text-\1-200$/,
      );
    }

    for (const id of templateIds) expect(TEMPLATE_ICONS[id], id).toBeDefined();
    expect(
      new Set(templateIds.map((id) => TEMPLATE_ICONS[id]?.icon)).size,
    ).toBe(12);
  });
  it("translates display metadata and uses instruction runtime for every template", () => {
    for (const t of templates) {
      expect(t.runtimeKey, t.id).toBe("library.discover2.runtime.instructions");
      expect(t.publisher).toBe("Ofox");
      expect(t.licenseKey).toBe("template");
      for (const locale of [zh, en, ja]) {
        for (const key of [
          t.titleKey,
          t.descriptionKey,
          t.preparationKey,
          `library.discover2.categories.${t.category}`,
        ]) {
          expect(key, t.id).toBeTruthy();
          expect(get(locale, key!), `${t.id}: ${key}`).toBeTruthy();
        }
      }
    }
  });
});
