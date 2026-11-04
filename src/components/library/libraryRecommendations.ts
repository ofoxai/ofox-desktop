import type { LibraryKind } from "@/lib/api/library";
import recommendedSkills from "./recommendedSkills.json";

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
  | "search"
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
  signIn?: boolean;
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

export function needsApiKey(recipe: LibraryRecommendation): boolean {
  return (
    recipe.kind === "mcp" &&
    [...recipe.initialDraft.environment, ...recipe.initialDraft.headers].some(
      (pair) => pair.value === "",
    )
  );
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
    publisher: "Ofox",
    licenseKey: "template",
  },
  chinese: {
    category: "communication",
    publisher: "Ofox",
    licenseKey: "template",
  },
  review: {
    category: "communication",
    publisher: "Ofox",
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
  // MCP metadata and descriptions adapted from magpie (MIT), © 2026 yetone.
  {
    ...base("exa", "Exa", "https://docs.exa.ai/reference/exa-mcp"),
    kind: "mcp",
    publisher: "Exa",
    category: "search",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.publicHttp",
    descriptionKey: "library.discover3.recipes.exa",
    initialDraft: createMcpDraft({
      name: "exa",
      transport: "http",
      url: "https://mcp.exa.ai/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.publicHttp"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "memory",
      "Memory",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/memory",
    ),
    kind: "mcp",
    publisher: "Model Context Protocol",
    category: "development",
    licenseKey: "sourceTerms",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.node",
    preparationKey: "library.discover3.preparation.node",
    descriptionKey: "library.discover3.recipes.memory",
    initialDraft: createMcpDraft({
      name: "memory",
      command: "npx",
      argsText: "-y\n@modelcontextprotocol/server-memory",
    }),
    dependencyKeys: ["library.discover3.preparation.node"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base(
      "sequential-thinking",
      "Sequential Thinking",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/sequentialthinking",
    ),
    kind: "mcp",
    publisher: "Model Context Protocol",
    category: "development",
    licenseKey: "sourceTerms",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.node",
    preparationKey: "library.discover3.preparation.node",
    descriptionKey: "library.discover3.recipes.sequential-thinking",
    initialDraft: createMcpDraft({
      name: "sequential-thinking",
      command: "npx",
      argsText: "-y\n@modelcontextprotocol/server-sequential-thinking",
    }),
    dependencyKeys: ["library.discover3.preparation.node"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base(
      "git",
      "Git",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/git",
    ),
    kind: "mcp",
    publisher: "Model Context Protocol",
    category: "development",
    licenseKey: "mit",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.uv",
    preparationKey: "library.discover3.preparation.uv",
    descriptionKey: "library.discover3.recipes.git",
    initialDraft: createMcpDraft({
      name: "git",
      command: "uvx",
      argsText: "mcp-server-git",
    }),
    dependencyKeys: ["library.discover3.preparation.uv"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base(
      "time",
      "Time",
      "https://github.com/modelcontextprotocol/servers/tree/main/src/time",
    ),
    kind: "mcp",
    publisher: "Model Context Protocol",
    category: "development",
    licenseKey: "mit",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.uv",
    preparationKey: "library.discover3.preparation.uv",
    descriptionKey: "library.discover3.recipes.time",
    initialDraft: createMcpDraft({
      name: "time",
      command: "uvx",
      argsText: "mcp-server-time",
    }),
    dependencyKeys: ["library.discover3.preparation.uv"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base("sentry", "Sentry", "https://docs.sentry.io/product/sentry-mcp/"),
    kind: "mcp",
    publisher: "Sentry",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.sentry",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "sentry",
      transport: "http",
      url: "https://mcp.sentry.dev/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base("linear", "Linear", "https://linear.app/docs/mcp"),
    kind: "mcp",
    publisher: "Linear",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.linear",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "linear",
      transport: "http",
      url: "https://mcp.linear.app/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base("notion", "Notion", "https://developers.notion.com/docs/mcp"),
    kind: "mcp",
    publisher: "Notion",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.notion",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "notion",
      transport: "http",
      url: "https://mcp.notion.com/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "atlassian",
      "Atlassian",
      "https://www.atlassian.com/platform/remote-mcp-server",
    ),
    kind: "mcp",
    publisher: "Atlassian",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.atlassian",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "atlassian",
      transport: "http",
      url: "https://mcp.atlassian.com/v1/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "supabase",
      "Supabase",
      "https://supabase.com/docs/guides/getting-started/mcp",
    ),
    kind: "mcp",
    publisher: "Supabase",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.supabase",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "supabase",
      transport: "http",
      url: "https://mcp.supabase.com/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base("stripe", "Stripe", "https://docs.stripe.com/mcp"),
    kind: "mcp",
    publisher: "Stripe",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.stripe",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "stripe",
      transport: "http",
      url: "https://mcp.stripe.com",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base("vercel", "Vercel", "https://vercel.com/docs/mcp/vercel-mcp"),
    kind: "mcp",
    publisher: "Vercel",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.vercel",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "vercel",
      transport: "http",
      url: "https://mcp.vercel.com",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base("neon", "Neon", "https://neon.com/docs/ai/neon-mcp-server"),
    kind: "mcp",
    publisher: "Neon",
    category: "development",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.signIn",
    descriptionKey: "library.discover3.recipes.neon",
    signIn: true,
    initialDraft: createMcpDraft({
      name: "neon",
      transport: "http",
      url: "https://mcp.neon.tech/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.signIn"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "cloudflare-docs",
      "Cloudflare Docs",
      "https://github.com/cloudflare/mcp-server-cloudflare",
    ),
    kind: "mcp",
    publisher: "Cloudflare",
    category: "docs",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.publicHttp",
    descriptionKey: "library.discover3.recipes.cloudflare-docs",
    initialDraft: createMcpDraft({
      name: "cloudflare-docs",
      transport: "http",
      url: "https://docs.mcp.cloudflare.com/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.publicHttp"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "huggingface",
      "Hugging Face",
      "https://huggingface.co/settings/mcp",
    ),
    kind: "mcp",
    publisher: "Hugging Face",
    category: "search",
    licenseKey: "service",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.http",
    preparationKey: "library.discover3.preparation.publicHttp",
    descriptionKey: "library.discover3.recipes.huggingface",
    initialDraft: createMcpDraft({
      name: "huggingface",
      transport: "http",
      url: "https://huggingface.co/mcp",
    }),
    dependencyKeys: ["library.discover3.preparation.publicHttp"],
    permissionKeys: ["library.discover3.permissions.remote"],
  },
  {
    ...base(
      "brave-search",
      "Brave Search",
      "https://github.com/brave/brave-search-mcp-server",
    ),
    kind: "mcp",
    publisher: "Brave",
    category: "search",
    licenseKey: "mit",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.node",
    preparationKey: "library.discover3.preparation.apiKey",
    descriptionKey: "library.discover3.recipes.brave-search",
    initialDraft: createMcpDraft({
      name: "brave-search",
      command: "npx",
      argsText: "-y\n@brave/brave-search-mcp-server",
      environment: [
        { id: "brave-search-key", name: "BRAVE_API_KEY", value: "" },
      ],
    }),
    dependencyKeys: ["library.discover3.preparation.apiKey"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base(
      "firecrawl",
      "Firecrawl",
      "https://github.com/firecrawl/firecrawl-mcp-server",
    ),
    kind: "mcp",
    publisher: "Firecrawl",
    category: "search",
    licenseKey: "mit",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.node",
    preparationKey: "library.discover3.preparation.apiKey",
    descriptionKey: "library.discover3.recipes.firecrawl",
    initialDraft: createMcpDraft({
      name: "firecrawl",
      command: "npx",
      argsText: "-y\nfirecrawl-mcp",
      environment: [
        { id: "firecrawl-key", name: "FIRECRAWL_API_KEY", value: "" },
      ],
    }),
    dependencyKeys: ["library.discover3.preparation.apiKey"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  {
    ...base("tavily", "Tavily", "https://github.com/tavily-ai/tavily-mcp"),
    kind: "mcp",
    publisher: "Tavily",
    category: "search",
    licenseKey: "mit",
    reviewedAt: "2026-11-05",
    runtimeKey: "library.discover2.runtime.node",
    preparationKey: "library.discover3.preparation.apiKey",
    descriptionKey: "library.discover3.recipes.tavily",
    initialDraft: createMcpDraft({
      name: "tavily",
      command: "npx",
      argsText: "-y\ntavily-mcp",
      environment: [{ id: "tavily-key", name: "TAVILY_API_KEY", value: "" }],
    }),
    dependencyKeys: ["library.discover3.preparation.apiKey"],
    permissionKeys: ["library.discover3.permissions.local"],
  },
  ...recommendedSkills.map(({ id: name, owner, repo, directory }) => ({
    ...base(
      name,
      name,
      `https://github.com/${owner}/${repo}/tree/main/${directory}`,
    ),
    kind: "skills" as const,
    repository: {
      owner,
      name: repo,
      branch: "main",
      directory,
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
      zh: "# 通用编程\n\n- 修改前先阅读相关代码和项目约定。\n- 优先完成当前需求，避免无关改动。\n- 对行为变化补充有意义的测试，并报告验证结果。\n- 说明尚未验证的部分和需要用户决定的问题。\n- 先明确验收条件，遇到会影响实现的歧义时说明假设。\n- 复用项目已有的组件和工具，保持命名与风格一致。\n- 检查失败路径和边界条件，保护现有数据与兼容性。\n- 交付时简要说明改了什么、为什么改以及验证范围。\n",
      en: "# General coding\n\n- Read the related code and project conventions before changing it.\n- Focus on the current request and keep unrelated changes separate.\n- Add meaningful tests for behavior changes and report the results.\n- State what remains unverified and which decisions need user input.\n- Clarify acceptance criteria and state assumptions that affect implementation.\n- Reuse existing components and tools with consistent names and style.\n- Check failure paths and edge cases, preserving data and compatibility.\n- When delivering, explain what changed, why, and the verification scope.\n",
      ja: "# 基本のコーディング\n\n- 変更する前に、関連コードとプロジェクトの規約を確認する。\n- 現在の依頼に集中し、無関係な変更は分ける。\n- 動作の変更には意味のあるテストを追加し、検証結果を報告する。\n- 未検証の項目とユーザーの判断が必要な点を明示する。\n- 受け入れ条件を明確にし、実装に影響する曖昧さには仮定を示す。\n- 既存の部品とツールを再利用し、命名と書式を揃える。\n- 失敗経路と境界条件を確認し、データと互換性を守る。\n- 完了時に変更内容、理由、検証範囲を簡潔に説明する。\n",
    },
  },
  {
    ...base("chinese", "Chinese communication"),
    kind: "instructions",
    titleKey: "library.discover.templates.chinese",
    contentByLanguage: {
      zh: "# 中文沟通\n\n- 默认使用简体中文，保留代码标识符和专有名称。\n- 先说明结果，再解释必要的原因和操作步骤。\n- 使用清楚、简短的句子，遇到专业术语给出易懂的说明。\n- 对不确定的信息明确说明，不编造验证结果。\n- 操作步骤按执行顺序排列，需要命令时给出可复制的示例。\n- 路径、命令和报错保留原文，必要时补充中文解释。\n- 用具体的对象和动作说明问题，避免含糊指代。\n- 只提供与当前问题相关的信息，不重复用户的描述。\n",
      en: "# Chinese communication\n\n- Use Simplified Chinese by default, preserving code identifiers and proper names.\n- Start with the result, then explain the necessary reasoning and steps.\n- Use clear, concise sentences and explain unfamiliar technical terms.\n- Mark uncertainty explicitly and never invent verification results.\n- Order operational steps by execution and provide copyable command examples when needed.\n- Preserve paths, commands, and errors verbatim, adding Chinese explanations when useful.\n- Describe problems with concrete objects and actions instead of vague references.\n- Keep information relevant to the current question without repeating the user description.\n",
      ja: "# 中国語でのコミュニケーション\n\n- 原則として簡体字中国語を使い、コードの識別子と固有名詞は保つ。\n- 最初に結果を述べ、必要な理由と操作手順を続ける。\n- 簡潔で分かりやすい文章を使い、専門用語には説明を添える。\n- 不確かな情報を明示し、検証結果を作り上げない。\n- 操作手順は実行順に並べ、必要なコマンドにはコピーできる例を示す。\n- パス、コマンド、エラーは原文を保ち、必要なら中国語で説明する。\n- 具体的な対象と動作で問題を説明し、曖昧な指示語を避ける。\n- 現在の質問に関係する情報に絞り、依頼内容を繰り返さない。\n",
    },
  },
  {
    ...base("review", "Code review"),
    kind: "instructions",
    titleKey: "library.discover.templates.review",
    contentByLanguage: {
      zh: "# 代码审查\n\n- 优先检查正确性、数据保护和兼容性问题。\n- 每个问题注明触发条件、影响和具体代码位置。\n- 区分已确认问题、待验证风险与风格建议。\n- 说明检查范围、测试证据和仍然存在的限制。\n- 阅读相关调用方和测试，理解预期行为后再下结论。\n- 检查错误路径、边界条件和用户数据是否得到保护。\n- 优先报告能采取行动的问题，避免没有依据的猜测。\n- 建议修复时说明需要补充的回归验证。\n",
      en: "# Code review\n\n- Prioritize correctness, data protection, and compatibility.\n- Describe the trigger, impact, and exact code location for each finding.\n- Separate confirmed bugs, risks needing verification, and style suggestions.\n- State the review scope, test evidence, and remaining limitations.\n- Read related callers and tests to understand expected behavior before concluding.\n- Check error paths, edge cases, and protection of user data.\n- Prioritize actionable findings and avoid unsupported speculation.\n- When suggesting a fix, identify the regression verification needed.\n",
      ja: "# コードレビュー\n\n- 正確性、データ保護、互換性の問題を優先して確認する。\n- 各指摘に発生条件、影響、具体的なコード位置を添える。\n- 確認済みの不具合、要検証のリスク、スタイル提案を区別する。\n- 確認範囲、テストの根拠、残る制限を明示する。\n- 関連する呼び出し元とテストを読み、期待する動作を理解して判断する。\n- エラー経路、境界条件、ユーザーデータの保護を確認する。\n- 対応可能な指摘を優先し、根拠のない推測を避ける。\n- 修正案には必要な回帰検証を添える。\n",
    },
  },
  {
    ...base("tdd", "Test first"),
    kind: "instructions",
    titleKey: "library.discover.templates.tdd",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 测试先行\n\n- 先明确要改变的行为和验收条件。\n- 先写一个能体现该行为的测试。\n- 运行测试，确认它因预期原因失败。\n- 若失败来自环境或语法，先修正测试。\n- 只写让当前失败测试通过的最少代码。\n- 再次运行测试，确认新行为和相关旧行为通过。\n- 在测试保护下整理命名、结构和重复代码。\n- 整理后重新验证，再开始下一个行为。\n",
      en: "# Test first\n\n- Define the behavior to change and its acceptance criteria.\n- Write a test that demonstrates that behavior first.\n- Run it and confirm it fails for the expected reason.\n- Fix the test first if syntax or environment caused the failure.\n- Write only the code needed to pass the failing test.\n- Run tests again to verify the new behavior and related existing behavior.\n- Refactor naming, structure, and duplication under test coverage.\n- Verify the refactoring before starting the next behavior.\n",
      ja: "# テスト先行\n\n- 変更する動作と受け入れ条件を明確にする。\n- その動作を示すテストを先に書く。\n- 実行して、想定した理由で失敗することを確認する。\n- 構文や環境が原因なら、先にテストを修正する。\n- 失敗したテストを通すための最小限のコードを書く。\n- 再実行し、新しい動作と関連する既存の動作を確認する。\n- テストで保護しながら命名、構造、重複を整理する。\n- 整理後も検証し、次の動作に進む。\n",
    },
  },
  {
    ...base("debugging", "Find the root cause"),
    kind: "instructions",
    titleKey: "library.discover.templates.debugging",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 先找根因\n\n- 先记录问题的触发条件、输入和预期结果。\n- 用最小步骤稳定复现问题。\n- 完整阅读报错和堆栈，定位最早的异常。\n- 检查相关代码及最近改动，提出可验证的原因假设。\n- 每次只验证一个假设，并记录观察到的证据。\n- 找到根因后再修改，避免靠猜测堆叠补丁。\n- 为触发条件补充回归测试，验证修复及相关路径。\n- 同一问题尝试失败三次就停下来，总结证据并换思路。\n",
      en: "# Find the root cause\n\n- Record the trigger, input, and expected result first.\n- Reproduce the problem with the smallest reliable steps.\n- Read the full error and stack trace to locate the earliest failure.\n- Inspect related code and recent changes to form a testable hypothesis.\n- Test one hypothesis at a time and record the evidence.\n- Fix the root cause once established instead of stacking guesses.\n- Add a regression test for the trigger and verify related paths.\n- After three failed attempts on the same problem, stop, summarize evidence, and change approach.\n",
      ja: "# まず根本原因を探す\n\n- 発生条件、入力、期待する結果を先に記録する。\n- 最小限の手順で問題を安定して再現する。\n- エラーとスタック全体を読み、最初の異常を特定する。\n- 関連コードと最近の変更を調べ、検証可能な仮説を立てる。\n- 一度に一つの仮説を検証し、得られた証拠を記録する。\n- 根本原因が分かってから修正し、推測でパッチを重ねない。\n- 発生条件の回帰テストを追加し、関連経路も検証する。\n- 同じ問題で三回失敗したら止まり、証拠を整理して方針を変える。\n",
    },
  },
  {
    ...base("git-commits", "Commits and PRs"),
    kind: "instructions",
    titleKey: "library.discover.templates.git-commits",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 提交与 PR 规范\n\n- 每次提交聚焦一个能独立理解的改动。\n- 提交前检查差异，排除无关文件和敏感信息。\n- 使用 Conventional Commits 的类型和简洁主题。\n- 提交信息说明为什么改，必要时补充背景。\n- 仅在相关测试和检查通过后提交。\n- PR 描述先说明问题，再说明改动后的行为。\n- 写清验证命令、结果和仍未验证的范围。\n- 关联相关问题，并为界面变化提供前后对比。\n",
      en: "# Commits and PRs\n\n- Keep each commit focused on one independently understandable change.\n- Review the diff before committing and exclude unrelated files and secrets.\n- Use a Conventional Commits type and a concise subject.\n- Explain why the change is needed, adding context when useful.\n- Commit only after relevant tests and checks pass.\n- Start the PR description with the problem and resulting behavior.\n- State validation commands, results, and what remains unverified.\n- Link related issues and include before-and-after evidence for UI changes.\n",
      ja: "# コミットと PR の規約\n\n- 各コミットを単独で理解できる一つの変更に絞る。\n- コミット前に差分を確認し、無関係なファイルと秘密情報を除く。\n- Conventional Commits の種別と簡潔な件名を使う。\n- なぜ変更が必要かを述べ、必要に応じて背景を添える。\n- 関連するテストとチェックが通ってからコミットする。\n- PR の説明は問題と変更後の動作から始める。\n- 検証コマンド、結果、未検証の範囲を明記する。\n- 関連する課題を紐付け、UI 変更には変更前後の比較を添える。\n",
    },
  },
  {
    ...base("frontend", "Frontend interfaces"),
    kind: "instructions",
    titleKey: "library.discover.templates.frontend",
    category: "design",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 前端界面\n\n- 先阅读相邻界面和项目现有的组件约定。\n- 优先复用已有组件、布局和样式变量。\n- 使用语义化元素，为交互控件提供清楚的名称。\n- 检查键盘操作、焦点顺序和焦点可见性。\n- 使用主题色值，验证亮色与暗色模式的对比度。\n- 在窄屏下检查换行、溢出和操作区域。\n- 把用户可见文字放进 i18n，并补齐项目支持的语言。\n- 验证加载、空状态、错误和成功反馈。\n",
      en: "# Frontend interfaces\n\n- Read neighboring interfaces and existing component conventions first.\n- Reuse the project components, layouts, and style variables.\n- Use semantic elements and clear names for interactive controls.\n- Check keyboard operation, focus order, and visible focus.\n- Use theme colors and verify contrast in light and dark modes.\n- Check wrapping, overflow, and action areas on narrow screens.\n- Put visible text in i18n and cover the project supported languages.\n- Verify loading, empty, error, and success feedback.\n",
      ja: "# フロントエンド UI\n\n- まず周辺の画面と既存コンポーネントの規約を読む。\n- 既存の部品、レイアウト、スタイル変数を優先して使う。\n- 意味に合った要素を使い、操作部品に明確な名前を付ける。\n- キーボード操作、フォーカス順序、フォーカス表示を確認する。\n- テーマ色を使い、明色と暗色でコントラストを検証する。\n- 狭い画面で折り返し、はみ出し、操作領域を確認する。\n- 表示する文言は i18n に置き、対応言語を揃える。\n- 読み込み中、空、エラー、成功時の表示を検証する。\n",
    },
  },
  {
    ...base("docs-writing", "Documentation writing"),
    kind: "instructions",
    titleKey: "library.discover.templates.docs-writing",
    category: "communication",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 文档写作\n\n- 开头先给出结论或读者需要完成的操作。\n- 围绕读者的目标安排内容和步骤。\n- 使用简短、具体的句子，每段说明一件事。\n- 优先用常见词，必要术语在首次出现时解释。\n- 为命令示例写明前提、工作目录和必要参数。\n- 确认示例可以直接运行，并说明预期结果。\n- 明确区分事实、假设和未验证的内容。\n- 删去重复铺垫，更新受改动影响的相关说明。\n",
      en: "# Documentation writing\n\n- Start with the conclusion or the action the reader needs to take.\n- Organize content and steps around the reader goal.\n- Use short, concrete sentences and one idea per paragraph.\n- Prefer familiar words and explain necessary terms on first use.\n- State prerequisites, working directory, and required arguments for commands.\n- Verify examples can run directly and describe expected results.\n- Distinguish facts, assumptions, and unverified information.\n- Remove repetitive introductions and update related instructions affected by changes.\n",
      ja: "# ドキュメント執筆\n\n- 結論または読者が行う操作を冒頭に示す。\n- 読者の目的に沿って内容と手順を並べる。\n- 短く具体的な文を使い、一段落で一つのことを説明する。\n- 平易な言葉を選び、必要な専門用語は初出時に説明する。\n- コマンドには前提、作業ディレクトリ、必要な引数を示す。\n- 例をそのまま実行できることを確認し、期待する結果を示す。\n- 事実、仮定、未検証の内容を明確に分ける。\n- 重複する前置きを削り、変更に関係する説明も更新する。\n",
    },
  },
  {
    ...base("security", "Security checks"),
    kind: "instructions",
    titleKey: "library.discover.templates.security",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 安全检查\n\n- 提交前检查差异，确保不包含密钥、令牌或凭据。\n- 通过项目约定的安全存储读取敏感配置。\n- 在信任边界校验用户输入的类型、长度和允许值。\n- 使用参数化查询和安全参数传递，避免拼接可执行内容。\n- 按输出上下文转义内容，检查脚本和命令注入。\n- 在服务端校验身份、资源归属和操作权限。\n- 固定依赖版本，检查新增依赖的来源和安全公告。\n- 覆盖恶意输入与越权路径，报告发现及验证限制。\n",
      en: "# Security checks\n\n- Review diffs before committing to exclude keys, tokens, and credentials.\n- Read sensitive configuration through the project approved secure storage.\n- Validate user input types, lengths, and allowed values at trust boundaries.\n- Use parameterized queries and safe arguments instead of building executable strings.\n- Escape output for its context and check script and command injection.\n- Check identity, resource ownership, and action permissions on the server.\n- Pin dependency versions and check new dependency sources and security advisories.\n- Test malicious input and unauthorized paths, reporting findings and verification limits.\n",
      ja: "# セキュリティ確認\n\n- コミット前に差分を確認し、鍵、トークン、認証情報を除く。\n- 機密設定はプロジェクト指定の安全な保管先から読む。\n- 信頼境界で入力の型、長さ、許可される値を検証する。\n- 実行文字列の連結を避け、パラメータ化クエリと安全な引数を使う。\n- 出力先に合わせてエスケープし、スクリプトとコマンドの注入を調べる。\n- サーバー側で本人確認、リソースの所有関係、操作権限を検証する。\n- 依存バージョンを固定し、新規依存の出所と脆弱性情報を確認する。\n- 悪意ある入力と権限外操作をテストし、発見事項と検証の限界を報告する。\n",
    },
  },
  {
    ...base("refactor", "Small-step refactoring"),
    kind: "instructions",
    titleKey: "library.discover.templates.refactor",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 小步重构\n\n- 先明确重构范围和必须保持不变的外部行为。\n- 阅读调用方和已有测试，确认兼容性边界。\n- 缺少保护时，先补充当前行为的测试。\n- 一次只改一件事，避免同时加入功能或改变规则。\n- 把重构拆成可单独验证的小步骤。\n- 每一步都运行相关测试，保持可运行状态。\n- 若行为意外改变，先恢复或查明原因再继续。\n- 完成后检查差异，并报告行为保持的验证证据。\n",
      en: "# Small-step refactoring\n\n- Define the scope and external behavior that must remain unchanged.\n- Read callers and existing tests to establish compatibility boundaries.\n- Add tests for current behavior first where protection is missing.\n- Change one thing at a time without adding features or changing rules.\n- Split refactoring into small independently verifiable steps.\n- Run relevant tests after each step and keep the code runnable.\n- If behavior changes unexpectedly, restore it or diagnose before continuing.\n- Review the final diff and report evidence that behavior is preserved.\n",
      ja: "# 小さな単位でリファクタリング\n\n- 範囲と維持すべき外部動作を明確にする。\n- 呼び出し元と既存テストを読み、互換性の境界を確認する。\n- 保護が足りない箇所は、先に現在の動作のテストを追加する。\n- 一度に一つだけ変更し、機能追加や規則の変更を混ぜない。\n- 単独で検証できる小さな段階に分ける。\n- 各段階で関連テストを実行し、動く状態を保つ。\n- 意図せず動作が変わったら、戻すか原因を調べてから進む。\n- 最後に差分を確認し、動作を維持した検証根拠を報告する。\n",
    },
  },
  {
    ...base("plan-first", "Plan before coding"),
    kind: "instructions",
    titleKey: "library.discover.templates.plan-first",
    category: "development",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 先规划再动手\n\n- 先阅读相关代码、项目约定和已有资料。\n- 明确任务目标、范围和不做的事项。\n- 列出影响实现的未知信息与关键约束。\n- 把复杂任务拆成有顺序、可检查的步骤。\n- 为每一步定义具体的验收标准和验证方法。\n- 说明依赖关系、主要风险和需要选择的方案。\n- 把计划交给用户确认，确认后再写代码。\n- 执行时跟踪完成情况；范围或方案变化时先更新计划并确认。\n",
      en: "# Plan before coding\n\n- Read relevant code, project conventions, and existing material first.\n- Define the goal, scope, and exclusions.\n- List unknowns and key constraints that affect implementation.\n- Break complex work into ordered, checkable steps.\n- Define concrete acceptance criteria and validation for each step.\n- Explain dependencies, main risks, and choices that need a decision.\n- Ask the user to confirm the plan and write code after confirmation.\n- Track progress; update and confirm the plan when scope or approach changes.\n",
      ja: "# 実装前に計画する\n\n- 先に関連コード、プロジェクト規約、既存資料を読む。\n- 目標、範囲、対象外の事項を明確にする。\n- 実装に影響する未知の情報と主な制約を挙げる。\n- 複雑な作業を順序のある確認可能な手順に分ける。\n- 各手順に具体的な受け入れ条件と検証方法を定める。\n- 依存関係、主なリスク、判断が必要な選択肢を説明する。\n- 計画をユーザーに確認してもらい、確認後にコードを書く。\n- 進捗を追い、範囲や方針が変わる場合は計画を更新して確認する。\n",
    },
  },
  {
    ...base("concise", "Concise answers"),
    kind: "instructions",
    titleKey: "library.discover.templates.concise",
    category: "communication",
    publisher: "Ofox",
    licenseKey: "template",
    runtimeKey: "library.discover2.runtime.instructions",
    preparationKey: "library.discover2.preparation.coding",
    contentByLanguage: {
      zh: "# 简洁回答\n\n- 第一句直接回答问题或说明结果。\n- 只保留理解答案和采取行动需要的信息。\n- 不复述用户的问题，也不写空泛的开场白。\n- 用短句和具体词语表达，一段说明一件事。\n- 需要操作时给出可执行的步骤或命令。\n- 只有便于比较或扫描时才使用列表。\n- 简要说明影响结论的不确定性和限制。\n- 说清重点后结束，避免重复总结或无关建议。\n",
      en: "# Concise answers\n\n- Answer the question or state the result in the first sentence.\n- Keep information needed to understand the answer and act on it.\n- Do not repeat the question or add empty introductions.\n- Use short sentences and concrete words with one idea per paragraph.\n- Provide executable steps or commands when action is needed.\n- Use lists only when they improve comparison or scanning.\n- Briefly state uncertainty and limitations that affect the conclusion.\n- Stop after the key points without repetitive summaries or unrelated advice.\n",
      ja: "# 簡潔に答える\n\n- 最初の文で質問に答えるか、結果を述べる。\n- 理解と行動に必要な情報だけを残す。\n- ユーザーの質問を繰り返さず、空疎な前置きを書かない。\n- 短い文と具体的な言葉で、一段落に一つのことを書く。\n- 操作が必要なら、実行できる手順やコマンドを示す。\n- 比較や読み取りに役立つ場合だけリストを使う。\n- 結論に影響する不確かさと制限を簡潔に示す。\n- 要点を伝えたら終え、重複するまとめや無関係な提案を避ける。\n",
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
