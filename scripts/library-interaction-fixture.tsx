/** UI-only synthetic fixture. No real IPC, configuration writes, downloads or paid calls. */
import { createRoot } from "react-dom/client";
import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
  type LibraryInventory,
} from "@/lib/api/library";
import { LibraryPage } from "@/components/library/LibraryPage";
import { ShellProvider } from "@/components/shell/ShellContext";
import { Button } from "@/components/ui/button";
import "@/index.css";
void i18n.use(initReactI18next).init({
  lng: "zh",
  resources: {
    zh: { translation: zh },
    en: { translation: en },
    ja: { translation: ja },
  },
  interpolation: { escapeValue: false },
});
const internals = window as unknown as Record<string, unknown>;
internals.__TAURI_INTERNALS__ = {
  invoke: async (command: string) => {
    if (command === "plugin:event|listen") return 1;
    if (command === "plugin:event|unlisten") return;
    throw new Error("fixture: IPC blocked");
  },
  transformCallback: () => 1,
};
internals.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
  unregisterListener: () => undefined,
};
const targets = ["claude", "codex", "gemini", "opencode", "hermes"].map(
  (toolId) => ({
    toolId,
    available: toolId !== "hermes",
    discoveryStatus: "installed",
    path: `/sandbox/${toolId}`,
    targetGroupId: toolId,
    status: "unassigned",
    desired: false,
  }),
);
const db: Record<LibraryKind, LibraryData> = {
  instructions: {
    epoch: 1,
    instructions: [
      {
        id: "general",
        name: "团队通用编程规范",
        content:
          "# 通用编程\n\n先阅读现有代码，再修改。\n\n- 使用项目的测试\n- 保留用户配置\n\n```ts\nconst answer = 42;\n```",
        revision: 1,
      },
      {
        id: "review",
        name: "代码审查",
        content: "检查正确性、测试和数据保护。",
        revision: 1,
      },
    ],
    targets: targets.map((t) => ({
      ...t,
      desiredResourceId: t.toolId === "codex" ? "general" : null,
      readableResourceId: t.toolId === "codex" ? "general" : null,
      status: t.toolId === "codex" ? "applied" : t.status,
      userContentLines: 12,
      managedContentLines: t.toolId === "codex" ? 8 : 0,
      nativeState: "ready",
    })),
    operations: [],
  },
  mcp: {
    epoch: 1,
    servers: [
      {
        id: "context7",
        name: "Context7",
        displaySummary: "mcp.context7.com",
        iconId: "context7",
        revision: 1,
        transport: "http",
        serviceHost: "mcp.context7.com",
        credentialState: "none",
        fields: [],
        targets: targets.map((t) => ({
          ...t,
          desired: t.toolId === "codex",
          status: t.toolId === "codex" ? "applied" : t.status,
        })),
      },
      {
        id: "chrome",
        name: "Chrome DevTools 浏览器调试与中文长名称展示",
        displaySummary: "npx · chrome-devtools-mcp",
        iconId: "chrome-devtools",
        revision: 1,
        transport: "stdio",
        credentialState: "none",
        fields: [],
        targets,
      },
    ],
    operations: [],
  },
  skills: {
    epoch: 1,
    skills: [
      {
        id: "frontend-design",
        name: "frontend-design",
        description: "创建清晰、美观的前端页面与组件。",
        directory: "frontend-design",
        artifactHash: "fixture-hash",
        revision: 1,
        sourceState: "ready",
        sourceKind: "repository",
        repository: "anthropics/skills",
        commitSha: "a".repeat(40),
        targets,
      },
    ],
    operations: [],
  },
};
let nativeCollected = false;
libraryApi.summary = async () =>
  Object.fromEntries(
    Object.entries(db).map(([k, v]) => [
      k,
      (v.instructions ?? v.servers ?? v.skills ?? []).length,
    ]),
  ) as { instructions: number; mcp: number; skills: number };
libraryApi.list = async (kind) => structuredClone(db[kind]);
libraryApi.inventory = async (kind): Promise<LibraryInventory> => ({
  ticketId: `scan-${kind}`,
  kind,
  expiresAtMs: Date.now() + 600000,
  sources: [
    {
      sourceId: "native",
      sourceRef: "opaque",
      toolId: "codex",
      label:
        kind === "mcp"
          ? "nativeMcp"
          : kind === "skills"
            ? "nativeSkills"
            : "nativeInstructions",
      state: "ready",
      count: 1,
      locationKind: "file",
      displayPath: "/sandbox/native/config",
    },
  ],
  entries: [
    {
      entryId: `native-${kind}`,
      kind,
      name:
        kind === "mcp"
          ? "Playwright"
          : kind === "skills"
            ? "local-testing"
            : "本机个人指令",
      displaySummary: kind === "mcp" ? "npx · @playwright/mcp" : undefined,
      iconId: kind === "mcp" ? "playwright" : undefined,
      summary: kind === "mcp" ? "npx · @playwright/mcp" : "本机自有内容",
      sources: [{ sourceId: "native", toolId: "codex" }],
      matchedResourceIds:
        kind === "mcp" && nativeCollected ? ["native-playwright"] : [],
      conflict: false,
    },
    ...(kind === "mcp"
      ? [
          {
            entryId: "matched",
            kind,
            name: "Context7",
            displaySummary: "mcp.context7.com",
            iconId: "context7",
            sources: [{ sourceId: "native", toolId: "opencode" }],
            matchedResourceIds: ["context7"],
            conflict: false,
          },
        ]
      : []),
  ],
});
libraryApi.discardInventory = async () => true;
libraryApi.matchMcpRecipes = async () => [
  {
    recipeId: "context7",
    exactResourceIds: ["context7"],
    nativeEntryIds: [],
    similarNativeEntryIds: [],
  },
];
libraryApi.matchBuiltinRecipes = async () => [];
libraryApi.importInventory = async (_ticket, ids) => {
  if (ids[0] !== "native-mcp")
    throw new Error("fixture: this item is read-only");
  nativeCollected = true;
  db.mcp.servers!.push({
    ...db.mcp.servers![1],
    id: "native-playwright",
    name: "Playwright",
    displaySummary: "npx · @playwright/mcp",
    iconId: "playwright",
  });
  return {
    imported: 1,
    items: [
      {
        entryId: ids[0],
        resourceId: "native-playwright",
        name: "Playwright",
        status: "imported",
      },
    ],
  };
};
libraryApi.saveInstruction = async (set) => {
  const item = { ...set, revision: (set.revision ?? 0) + 1 };
  db.instructions.instructions = db.instructions
    .instructions!.filter((x) => x.id !== set.id)
    .concat(item);
  return item;
};
libraryApi.saveMcp = async (id, name) => {
  const previous = db.mcp.servers!.find((x) => x.id === id);
  db.mcp.servers = db.mcp
    .servers!.filter((x) => x.id !== id)
    .concat({
      ...previous,
      id,
      name,
      revision: (previous?.revision ?? 0) + 1,
      transport: previous?.transport ?? "stdio",
      credentialState: "none",
      fields: [],
      targets: previous?.targets ?? targets,
    });
};
libraryApi.skillDetail = async () => ({
  resourceId: "frontend-design",
  artifactHash: "fixture-hash",
  path: "SKILL.md",
  content:
    "# Frontend design\n\nCreate thoughtful interfaces with a clear hierarchy.\n\n## Workflow\n\n1. Understand the user.\n2. Build and verify.\n\n```tsx\nexport function Card() { return <article>Hello</article>; }\n```\n\n![Remote blocked](https://example.invalid/blocked.png)",
  files: [
    { path: "SKILL.md", isDirectory: false, sizeBytes: 256, executable: false },
  ],
});
libraryApi.prepareMcpCheck = async () => ({ operationId: "fixture-check" });
libraryApi.runMcpCheck = async () => ({
  operationId: "fixture-check",
  state: "ok",
  tools: [
    {
      name: "search_documentation",
      description: "Read public documentation for the selected library.",
    },
    {
      name: "resolve_library",
      description: "Find a public library identifier.",
    },
  ],
});
libraryApi.cancelMcpCheck = async () => true;
libraryApi.batches = async () => [];
libraryApi.previewBatch = async (kind, resources, tools) => ({
  operationId: `preview-${kind}`,
  kind,
  resources,
  status: "previewed",
  groups: [
    {
      groupId: "sandbox",
      tools: [
        ...new Set([
          ...tools,
          ...resources.flatMap((resource) => [
            ...(resource.addTools ?? []),
            ...(resource.withdrawTools ?? []),
          ]),
        ]),
      ],
      readers: ["codex"],
      path: "/sandbox/config",
      status: "pending",
      resourceIds: resources.map((r) => r.resourceId),
    },
  ],
});
libraryApi.runBatch = async () => {
  throw new Error("fixture: native writes blocked");
};
libraryApi.cancelBatch = async () => ({
  operationId: "fixture",
  kind: "mcp",
  status: "cancelled",
  resources: [],
  groups: [],
});
const client = new QueryClient({
  defaultOptions: { queries: { retry: false } },
});
function Fixture() {
  const [dark, setDark] = useState(false);
  return (
    <QueryClientProvider client={client}>
      <ShellProvider openSettings={() => undefined}>
        <main className="flex h-screen flex-col bg-canvas text-ink">
          <header className="flex flex-none items-center justify-between gap-2 border-b border-line px-4 py-2">
            <strong>OFox · Library</strong>
            <div className="flex gap-2">
              {["zh", "en", "ja"].map((l) => (
                <Button
                  variant="ghost"
                  key={l}
                  onClick={() => void i18n.changeLanguage(l)}
                >
                  {l}
                </Button>
              ))}
              <Button
                variant="secondary"
                onClick={() => {
                  setDark(!dark);
                  document.documentElement.classList.toggle("dark");
                }}
              >
                Theme
              </Button>
            </div>
          </header>
          <div className="min-h-0 flex-1 overflow-auto px-4 pb-4 pt-1">
            <LibraryPage />
          </div>
          <p className="flex-none border-t border-line px-4 py-1 text-note text-ink-muted">
            Synthetic UI sandbox · real configuration and paid calls blocked
          </p>
        </main>
      </ShellProvider>
    </QueryClientProvider>
  );
}
createRoot(document.getElementById("root")!).render(<Fixture />);
