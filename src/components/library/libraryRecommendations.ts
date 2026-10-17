import type { LibraryKind } from "@/lib/api/library";

export interface LibraryMcpPair {
  id: string;
  name: string;
  value: string;
}
export interface LibraryMcpDraft {
  identifier?: string;
  name: string;
  transport: "stdio" | "http" | "sse";
  command: string;
  argsText: string;
  /** Exact argv from a parsed command; absent for legacy newline-only drafts. */
  args?: string[];
  url: string;
  environment: LibraryMcpPair[];
  headers: LibraryMcpPair[];
  advancedFields?: { path: string[]; value: unknown }[];
  requiresDirectory?: boolean;
  directory?: string;
}
export type LibraryRecommendationCategory =
  | "docs"
  | "browser"
  | "files"
  | "development"
  | "design"
  | "communication";

interface RecommendationBase {
  id: string;
  kind: LibraryKind;
  name: string;
  titleKey?: string;
  descriptionKey: string;
  sourceUrl?: string;
  dependencyKeys: string[];
  permissionKeys: string[];
  toolIds: string[];
  referenceExample?: boolean;
  category?: LibraryRecommendationCategory;
  publisher?: string;
  reviewedAt?: string;
  licenseKey?: string;
  runtimeKey?: string;
  preparationKey?: string;
  authRequired?: boolean;
  checkMode?: "ofox" | "native";
}
export type LibraryRecommendation = RecommendationBase &
  (
    | { kind: "mcp"; initialDraft: LibraryMcpDraft }
    | {
        kind: "skills";
        repository: {
          owner: string;
          name: string;
          branch: string;
          directory: string;
        };
      }
    | {
        kind: "instructions";
        contentByLanguage: Record<"zh" | "en" | "ja", string>;
      }
  );
export function createMcpDraft(
  initial: Partial<LibraryMcpDraft> = {},
): LibraryMcpDraft {
  return {
    name: "",
    transport: "stdio",
    command: "",
    argsText: "",
    url: "",
    environment: [],
    headers: [],
    ...initial,
  };
}

const TARGETS = ["claude", "codex", "gemini", "opencode", "hermes"];
const RECIPE_METADATA: Record<
  string,
  Pick<RecommendationBase, "category" | "publisher" | "licenseKey">
> = {
  context7: { category: "docs", publisher: "Upstash", licenseKey: "mit" },
  "chrome-devtools": {
    category: "browser",
    publisher: "Chrome DevTools",
    licenseKey: "apache",
  },
  filesystem: {
    category: "files",
    publisher: "Model Context Protocol",
    licenseKey: "reference",
  },
  fetch: {
    category: "files",
    publisher: "Model Context Protocol",
    licenseKey: "reference",
  },
  "microsoft-learn": {
    category: "docs",
    publisher: "Microsoft",
    licenseKey: "service",
  },
  deepwiki: { category: "docs", publisher: "Cognition", licenseKey: "service" },
  playwright: {
    category: "browser",
    publisher: "Microsoft",
    licenseKey: "apache",
  },
  github: {
    category: "development",
    publisher: "GitHub",
    licenseKey: "mitService",
  },
  "frontend-design": {
    category: "design",
    publisher: "Anthropic",
    licenseKey: "skill",
  },
  "webapp-testing": {
    category: "browser",
    publisher: "Anthropic",
    licenseKey: "skill",
  },
  "mcp-builder": {
    category: "development",
    publisher: "Anthropic",
    licenseKey: "skill",
  },
  "skill-creator": {
    category: "development",
    publisher: "Anthropic",
    licenseKey: "skill",
  },
  "theme-factory": {
    category: "design",
    publisher: "Anthropic",
    licenseKey: "skill",
  },
  "canvas-design": {
    category: "design",
    publisher: "Anthropic",
    licenseKey: "skillFonts",
  },
  coding: {
    category: "development",
    publisher: "OFox",
    licenseKey: "template",
  },
  chinese: {
    category: "communication",
    publisher: "OFox",
    licenseKey: "template",
  },
  review: {
    category: "communication",
    publisher: "OFox",
    licenseKey: "template",
  },
};
const base = (id: string, name: string, sourceUrl?: string) => ({
  id,
  name,
  sourceUrl,
  toolIds: TARGETS,
  descriptionKey: `library.discover.recipes.${id}`,
  dependencyKeys: [] as string[],
  permissionKeys: [] as string[],
  ...RECIPE_METADATA[id],
  reviewedAt: "2026-10-17",
  runtimeKey: `library.discover2.runtime.${["context7", "microsoft-learn", "deepwiki", "github"].includes(id) ? "http" : ["chrome-devtools", "filesystem", "playwright"].includes(id) ? "node" : id === "fetch" ? "uv" : ["coding", "chinese", "review"].includes(id) ? "instructions" : "skill"}`,
  preparationKey: `library.discover2.preparation.${id}`,
  checkMode: (["context7", "microsoft-learn", "deepwiki"].includes(id)
    ? "ofox"
    : "native") as "ofox" | "native",
});

/** Maintained recipes only. Browsing or selecting one never runs or downloads it. */
export const LIBRARY_RECOMMENDATIONS: readonly LibraryRecommendation[] = [
  {
    ...base("context7", "Context7", "https://github.com/upstash/context7"),
    kind: "mcp",
    initialDraft: createMcpDraft({
      name: "context7",
      transport: "http",
      url: "https://mcp.context7.com/mcp",
    }),
    dependencyKeys: ["library.discover.requirements.context7"],
    permissionKeys: ["library.discover.permissions.remoteDocs"],
  },
  {
    ...base(
      "chrome-devtools",
      "Chrome DevTools",
      "https://github.com/ChromeDevTools/chrome-devtools-mcp",
    ),
    kind: "mcp",
    initialDraft: createMcpDraft({
      name: "chrome-devtools",
      command: "npx",
      argsText:
        "-y\nchrome-devtools-mcp@latest\n--no-usage-statistics\n--no-performance-crux",
    }),
    dependencyKeys: ["library.discover.requirements.chrome"],
    permissionKeys: ["library.discover.permissions.browser"],
  },
  {
    ...base(
      "filesystem",
      "Filesystem",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/filesystem",
    ),
    kind: "mcp",
    referenceExample: true,
    initialDraft: createMcpDraft({
      name: "filesystem",
      command: "npx",
      argsText: "-y\n@modelcontextprotocol/server-filesystem",
      requiresDirectory: true,
      directory: "",
    }),
    dependencyKeys: ["library.discover.requirements.filesystem"],
    permissionKeys: ["library.discover.permissions.filesystem"],
  },
  {
    ...base(
      "fetch",
      "Fetch",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/fetch",
    ),
    kind: "mcp",
    referenceExample: true,
    initialDraft: createMcpDraft({
      name: "fetch",
      command: "uvx",
      argsText: "mcp-server-fetch",
    }),
    dependencyKeys: ["library.discover.requirements.fetch"],
    permissionKeys: ["library.discover.permissions.web"],
  },
  {
    ...base(
      "microsoft-learn",
      "Microsoft Learn",
      "https://learn.microsoft.com/en-us/training/support/mcp",
    ),
    kind: "mcp",
    descriptionKey: "library.discover2.recipes.microsoft-learn",
    initialDraft: createMcpDraft({
      name: "microsoft-learn",
      transport: "http",
      url: "https://learn.microsoft.com/api/mcp",
    }),
    dependencyKeys: ["library.discover2.requirements.publicHttp"],
    permissionKeys: ["library.discover2.permissions.microsoft-learn"],
  },
  {
    ...base(
      "deepwiki",
      "DeepWiki",
      "https://docs.devin.ai/work-with-devin/deepwiki-mcp",
    ),
    kind: "mcp",
    descriptionKey: "library.discover2.recipes.deepwiki",
    initialDraft: createMcpDraft({
      name: "deepwiki",
      transport: "http",
      url: "https://mcp.deepwiki.com/mcp",
    }),
    dependencyKeys: ["library.discover2.requirements.publicHttp"],
    permissionKeys: ["library.discover2.permissions.deepwiki"],
  },
  {
    ...base(
      "playwright",
      "Playwright",
      "https://github.com/microsoft/playwright-mcp",
    ),
    kind: "mcp",
    descriptionKey: "library.discover2.recipes.playwright",
    initialDraft: createMcpDraft({
      name: "playwright",
      command: "npx",
      argsText: "-y\n@playwright/mcp@latest",
    }),
    dependencyKeys: ["library.discover2.requirements.playwright"],
    permissionKeys: ["library.discover2.permissions.playwright"],
  },
  {
    ...base(
      "github",
      "GitHub",
      "https://github.com/github/github-mcp-server/blob/main/docs/remote-server.md",
    ),
    kind: "mcp",
    descriptionKey: "library.discover2.recipes.github",
    authRequired: true,
    initialDraft: createMcpDraft({
      name: "github",
      transport: "http",
      url: "https://api.githubcopilot.com/mcp/readonly",
    }),
    dependencyKeys: ["library.discover2.requirements.github"],
    permissionKeys: ["library.discover2.permissions.github"],
  },
  ...[
    "frontend-design",
    "webapp-testing",
    "mcp-builder",
    "skill-creator",
    "theme-factory",
    "canvas-design",
  ].map((name) => ({
    ...base(
      name,
      name,
      `https://github.com/anthropics/skills/tree/main/skills/${name}`,
    ),
    kind: "skills" as const,
    repository: {
      owner: "anthropics",
      name: "skills",
      branch: "main",
      directory: `skills/${name}`,
    },
    descriptionKey: ["theme-factory", "canvas-design"].includes(name)
      ? `library.discover2.recipes.${name}`
      : `library.discover.recipes.${name}`,
    dependencyKeys: [
      `library.${["theme-factory", "canvas-design"].includes(name) ? "discover2" : "discover"}.requirements.${name}`,
    ],
    permissionKeys: ["library.discover.permissions.skillFiles"],
  })),
  {
    ...base("coding", "General coding"),
    kind: "instructions",
    titleKey: "library.discover.templates.coding",
    contentByLanguage: {
      zh: "# 通用编程\n\n- 修改前先阅读相关代码和项目约定。\n- 优先完成当前需求，避免无关改动。\n- 对行为变化补充有意义的测试，并报告验证结果。\n- 说明尚未验证的部分和需要用户决定的问题。\n",
      en: "# General coding\n\n- Read the related code and project conventions before changing it.\n- Focus on the current request and keep unrelated changes separate.\n- Add meaningful tests for behavior changes and report the results.\n- State what remains unverified and which decisions need user input.\n",
      ja: "# 基本のコーディング\n\n- 変更する前に、関連コードとプロジェクトの規約を確認する。\n- 現在の依頼に集中し、無関係な変更は分ける。\n- 動作の変更には意味のあるテストを追加し、検証結果を報告する。\n- 未検証の項目とユーザーの判断が必要な点を明示する。\n",
    },
  },
  {
    ...base("chinese", "Chinese communication"),
    kind: "instructions",
    titleKey: "library.discover.templates.chinese",
    contentByLanguage: {
      zh: "# 中文沟通\n\n- 默认使用简体中文，保留代码标识符和专有名称。\n- 先说明结果，再解释必要的原因和操作步骤。\n- 使用清楚、简短的句子，遇到专业术语给出易懂的说明。\n- 对不确定的信息明确说明，不编造验证结果。\n",
      en: "# Chinese communication\n\n- Use Simplified Chinese by default, preserving code identifiers and proper names.\n- Start with the result, then explain the necessary reasoning and steps.\n- Use clear, concise sentences and explain unfamiliar technical terms.\n- Mark uncertainty explicitly and never invent verification results.\n",
      ja: "# 中国語でのコミュニケーション\n\n- 原則として簡体字中国語を使い、コードの識別子と固有名詞は保つ。\n- 最初に結果を述べ、必要な理由と操作手順を続ける。\n- 簡潔で分かりやすい文章を使い、専門用語には説明を添える。\n- 不確かな情報を明示し、検証結果を作り上げない。\n",
    },
  },
  {
    ...base("review", "Code review"),
    kind: "instructions",
    titleKey: "library.discover.templates.review",
    contentByLanguage: {
      zh: "# 代码审查\n\n- 优先检查正确性、数据保护和兼容性问题。\n- 每个问题注明触发条件、影响和具体代码位置。\n- 区分已确认问题、待验证风险与风格建议。\n- 说明检查范围、测试证据和仍然存在的限制。\n",
      en: "# Code review\n\n- Prioritize correctness, data protection, and compatibility.\n- Describe the trigger, impact, and exact code location for each finding.\n- Separate confirmed bugs, risks needing verification, and style suggestions.\n- State the review scope, test evidence, and remaining limitations.\n",
      ja: "# コードレビュー\n\n- 正確性、データ保護、互換性の問題を優先して確認する。\n- 各指摘に発生条件、影響、具体的なコード位置を添える。\n- 確認済みの不具合、要検証のリスク、スタイル提案を区別する。\n- 確認範囲、テストの根拠、残る制限を明示する。\n",
    },
  },
];

export function cloneRecommendationDraft(
  recipe: LibraryRecommendation,
): LibraryMcpDraft {
  if (recipe.kind !== "mcp") throw new Error("notMcp");
  return {
    ...recipe.initialDraft,
    args: recipe.initialDraft.args ? [...recipe.initialDraft.args] : undefined,
    environment: recipe.initialDraft.environment.map((pair) => ({ ...pair })),
    headers: recipe.initialDraft.headers.map((pair) => ({ ...pair })),
    advancedFields: recipe.initialDraft.advancedFields?.map((field) => ({
      ...field,
      path: [...field.path],
    })),
  };
}
