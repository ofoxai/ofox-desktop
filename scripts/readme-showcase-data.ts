import type {
  AssistantBootstrap,
  AssistantModel,
  AssistantSession,
  AssistantSessionDetail,
  AssistantUpdateRequest,
} from "@/lib/api/assistant";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import type { ToolUpdateInfo } from "@/lib/api/toolUpdates";
import type { ModelProfile } from "@/lib/api/modelProfiles";
import {
  readmeImageModel,
  readmeVideoModel,
  readmeSpeechModel,
} from "./readme-media-catalog";

export const BOUND_TOOLS = ["claude", "codex", "opencode", "gemini", "hermes"];
export const account: OfoxAccount = {
  user: {
    id: "readme-demo",
    name: "Ofox Demo",
    email: "demo@example.com",
    org_id: "demo-workspace",
    avatar_url: null,
    role: "member",
    balance: { balance: 42.8, total: 50, used: 7.2, currency: "USD" },
  },
  retrying: false,
  retryAttempt: 0,
  loadFailed: false,
  balanceRefreshing: false,
  loadUser: async () => account.user,
  retryUser: async () => undefined,
  refreshBalance: async () => undefined,
};
const modelByTool: Record<string, string> = {
  claude: "anthropic/claude-sonnet-4.6",
  codex: "openai/gpt-5.4",
  opencode: "google/gemini-3.1-pro-preview",
  gemini: "google/gemini-3.1-pro-preview",
  hermes: "deepseek/deepseek-v4-flash",
};
export const binding = {
  status: "configured" as const,
  message: null,
  missingFiles: [],
  modifiedFields: [],
  envOverrides: [],
};
export const toolSnapshots: ToolModelSnapshot[] = BOUND_TOOLS.map((toolId) => ({
  toolId,
  writeTarget: toolId,
  revision: `demo-${toolId}`,
  binding,
  managedSelection: { kind: "single", model: modelByTool[toolId] },
  observedSelection: { model: modelByTool[toolId], source: "native" },
  canWrite: true,
  scope: ["global"],
  ...(["claude", "codex", "hermes"].includes(toolId)
    ? {
        effort: {
          value: "medium",
          levels: ["low", "medium", "high"],
          defaultLevel: "medium",
        },
      }
    : {}),
}));
const versions: Record<string, string> = {
  claude: "2.1.0",
  codex: "0.100.0",
  opencode: "1.1.0",
  gemini: "0.30.0",
  hermes: "0.10.0",
};
export const toolVersions: ToolUpdateInfo[] = BOUND_TOOLS.map((name) => ({
  name,
  version: versions[name],
  error: null,
  installationKind: "cli",
  installationStatus: "installed",
  latest_version: versions[name],
  update_status: "current",
  update_source: "npm",
  update_supported: true,
  update_reason: null,
  executable_path: `/demo/bin/${name}`,
}));
export const profiles: ModelProfile[] = [
  {
    id: "balanced",
    name: "Balanced workspace",
    revision: "demo-1",
    match: "inUse",
    targets: toolSnapshots.map((tool) => ({
      target: tool.toolId,
      selection: tool.managedSelection,
    })),
  },
  {
    id: "deep-work",
    name: "Deep work",
    revision: "demo-1",
    match: "different",
    targets: toolSnapshots.map((tool) => ({
      target: tool.toolId,
      selection: ["claude", "opencode", "hermes"].includes(tool.toolId)
        ? { kind: "single", model: "anthropic/claude-opus-4.6" }
        : tool.managedSelection,
    })),
  },
];
export const demoCatalog = [
  ["anthropic/claude-sonnet-4.6", "Claude Sonnet 4.6", "anthropic"],
  ["anthropic/claude-opus-4.6", "Claude Opus 4.6", "anthropic"],
  ["openai/gpt-5.4", "GPT-5.4", "openai"],
  ["google/gemini-3.1-pro-preview", "Gemini 3.1 Pro", "google"],
  ["deepseek/deepseek-v4-flash", "DeepSeek V4 Flash", "deepseek"],
].map(([id, name, ownedBy]) => ({
  id,
  name,
  ownedBy,
  pricingPrompt: null,
  supportedEndpoints: [
    "/v1/chat/completions",
    "/v1/responses",
    "/anthropic/v1/messages",
  ],
  supportedParameters: ["tools", "reasoning"],
  inputModalities: ["text", "image"],
  outputModalities: ["text"],
}));
const chatModel = (
  id: string,
  name: string,
  vendor: string,
): AssistantModel => ({
  id,
  name,
  vendor,
  shortName: name,
  kind: "chat",
  contextWindow: 200000,
  maxOutputTokens: 16000,
  supportsImages: true,
  inputModalities: ["text", "image"],
  supportedEndpoints: ["chat/completions"],
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
  protocol: "openai-completions",
  tools: true,
  reasoning: true,
  thinkingLevels: ["low", "medium", "high"],
});
export const assistantBootstrap: AssistantBootstrap = {
  owner: "a".repeat(64),
  authorityGeneration: 1,
  runtime: { available: true, reason: null, version: "sample-data" },
  models: [
    chatModel("anthropic/claude-sonnet-4.6", "Claude Sonnet 4.6", "anthropic"),
    chatModel("openai/gpt-5.4", "GPT-5.4", "openai"),
    chatModel("google/gemini-3.1-pro-preview", "Gemini 3.1 Pro", "google"),
    readmeImageModel,
    readmeVideoModel,
  ],
  audioModels: [readmeSpeechModel],
  defaultModelId: "anthropic/claude-sonnet-4.6",
  modelUnavailableReason: null,
  key: { status: "ready" },
  search: { available: true, unitPriceUsd: "0.014", model: "sample-search" },
};
export const assistantSessions: AssistantSession[] = [
  "Aurora — architecture review",
  "A better release checklist",
  "Explain a tricky function",
  "Ideas for a product launch",
].map((title, index) => ({
  id: `readme-session-${index}`,
  title,
  titleSource: "user",
  assistantId: "ofox",
  capabilities: [],
  modelId: assistantBootstrap.defaultModelId!,
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: Date.now() - index * 86400000,
  updatedAt: Date.now() - index * 86400000,
  deletedAt: null,
  pinnedAt: index === 0 ? Date.now() : null,
  activeRun: null,
}));
export const README_MEDIA_SESSION_ID = "readme-media-session";
const mediaSession: AssistantSession = {
  ...assistantSessions[0],
  id: README_MEDIA_SESSION_ID,
  title: "Studio — image, video, and voice",
  draft:
    "Create a watercolor landscape, animate a gentle camera move, and add a warm voice-over.",
  media: {
    hints: ["image", "video", "speech"],
    image: {
      model: readmeImageModel.id,
      size: "1536x1024",
      quality: "high",
      n: 1,
    },
    video: {
      model: readmeVideoModel.id,
      duration: 5,
      resolution: "720p",
      aspectRatio: "16:9",
      generateAudio: true,
    },
    speech: { model: readmeSpeechModel.id, voice: "Warm", format: "wav" },
  },
  pinnedAt: Date.now(),
};
assistantSessions.push(mediaSession);
export function assistantDetail(id: string): AssistantSessionDetail {
  if (id === README_MEDIA_SESSION_ID)
    return {
      session: structuredClone(mediaSession),
      attachments: [],
      messages: [],
      usage: { inputTokens: 0, outputTokens: 0, estimatedCostUsd: null },
      permissionMode: "default",
    };
  const session =
    assistantSessions.find((item) => item.id === id) ?? assistantSessions[0];
  return {
    session,
    attachments: [],
    usage: {
      inputTokens: 1240,
      outputTokens: 420,
      usageCoverage: "complete",
      estimatedCostUsd: null,
    },
    permissionMode: "default",
    messages: [
      {
        id: "demo-question",
        role: "user",
        createdAt: Date.now() - 120000,
        content:
          "Help me plan a clean architecture for Aurora, a small team knowledge app. Keep it simple and easy to test.",
        status: "completed",
      },
      {
        id: "demo-answer",
        role: "assistant",
        createdAt: Date.now() - 110000,
        modelId: session.modelId,
        status: "completed",
        content:
          "## A small core, clear boundaries\n\nStart with three layers. Keep the domain independent of the database and the UI.\n\n| Layer | Owns | Example |\n| --- | --- | --- |\n| Domain | Rules and types | `Note`, `Workspace` |\n| Application | User workflows | `CreateNote` |\n| Adapters | Storage and transport | SQLite, HTTP |\n\n```ts\ninterface NoteRepository {\n  find(id: string): Promise<Note | null>;\n  save(note: Note): Promise<void>;\n}\n```\n\n**First milestone:** create a note, search it, and share it with a teammate. Test each workflow with an in-memory repository before adding persistence.",
      },
    ],
  };
}

export function handleToolsIpc(
  command: string,
  payload: Record<string, unknown>,
) {
  switch (command) {
    case "plugin:window|is_visible":
      return true;
    case "plugin:window|is_minimized":
      return false;
    case "set_window_theme":
      return;
    case "get_tool_versions":
      return toolVersions;
    case "get_tool_binding_status":
      return binding;
    case "ofox_list_api_keys":
      return BOUND_TOOLS.map((tool) => ({
        tool,
        keyId: `demo-${tool}`,
        alias: "Aurora workspace",
      }));
    case "ofox_get_user_info":
    case "ofox_refresh_user_info":
      return account.user;
    case "ofox_reconcile_tool_keys":
      return { switched: [], skipped: [], failed: [] };
    case "get_tool_install_capabilities":
      return BOUND_TOOLS;
    case "get_install_catalog_inventory":
      return [
        {
          toolId: "chatgpt",
          status: "installed",
          version: "1.0",
          path: "/demo/apps/ChatGPT.app",
        },
        {
          toolId: "openclaw",
          status: "installed",
          version: "2026.10.0",
          path: "/demo/bin/openclaw",
        },
      ];
    case "model_selection_snapshot":
      return { tools: toolSnapshots };
    case "model_profiles_list":
      return profiles;
    case "model_profiles_preview": {
      const profile = profiles.find((item) => item.id === payload.id);
      if (!profile) throw new Error("Unknown sample profile");
      const items = profile.targets.map((target) => {
        const before =
          toolSnapshots.find((tool) => tool.toolId === target.target)
            ?.managedSelection ?? null;
        return {
          target: target.target,
          before,
          after: target.selection,
          status:
            JSON.stringify(before) === JSON.stringify(target.selection)
              ? "unchanged"
              : "ready",
          scope: ["global"],
        };
      });
      return {
        planToken: "sample-plan",
        canApply: items.some((item) => item.status === "ready"),
        items,
      };
    }
    case "fetch_ofox_models":
      return demoCatalog;
    case "ofox_get_apex":
      return "ofox.ai";
    case "ofox_get_apex_state":
      return { apex: "ofox.ai", pinned: true, detected: true };
    case "check_for_updates":
      return {
        hasUpdate: false,
        currentVersion: "1.5.0",
        latestVersion: "1.5.0",
      };
    case "assistant_bootstrap":
      return assistantBootstrap;
    case "assistant_get_preferences":
      return { instructions: "", autoTitle: true, notifyOnFinish: false };
    case "assistant_list_sessions":
      return {
        sessions: payload.deleted ? [] : assistantSessions,
        unreadable: [],
        trashCount: 0,
        matches: {},
      };
    case "assistant_get_session":
      return assistantDetail(String(payload.sessionId));
    case "assistant_update_session": {
      const request = payload.request as
        | Partial<AssistantUpdateRequest>
        | undefined;
      if (request?.sessionId !== README_MEDIA_SESSION_ID)
        throw new Error(
          "README sandbox updates only the fictional media draft",
        );
      const allowed = new Set([
        "sessionId",
        "expectedOwner",
        "expectedGeneration",
        "draft",
        "media",
        "modelId",
        "modelPicked",
        "thinkingLevel",
      ]);
      if (Object.keys(request).some((key) => !allowed.has(key)))
        throw new Error("README sandbox rejects non-draft session changes");
      // This is only the fixture's page-local object. No generation, credential,
      // settings, filesystem, or configuration-writing API is called.
      if (typeof request.draft === "string") mediaSession.draft = request.draft;
      if (request.media) mediaSession.media = structuredClone(request.media);
      if (typeof request.modelId === "string")
        mediaSession.modelId = request.modelId;
      if (request.thinkingLevel !== undefined)
        mediaSession.thinkingLevel =
          request.thinkingLevel === "default" ? null : request.thinkingLevel;
      return structuredClone(mediaSession);
    }
    case "assistant_local_changes":
      return [];
    case "assistant_usage_summary":
      return {
        modelRequests: 4,
        inputTokens: 2400,
        outputTokens: 820,
        estimatedCostUsd: null,
      };
    default:
      throw new Error(`README sandbox has no handler: ${command}`);
  }
}
