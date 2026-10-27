import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type AssistantKind = "general" | "configuration";
export type AssistantPermission =
  | "readOnly"
  | "ask"
  | "autoConfig"
  | "fullAccess";

export interface AssistantModel {
  id: string;
  name: string;
  contextWindow?: number | null;
  maxOutputTokens?: number | null;
  supportsImages?: boolean;
  inputPricePerMillion?: string | null;
  outputPricePerMillion?: string | null;
}

export type AssistantActionSupport =
  | "automatic"
  | "confirmation"
  | "toolsPage"
  | "unsupported";
export interface AssistantToolCapabilities {
  tools: {
    tool: string;
    actions: {
      action: string;
      supportByMode: Record<AssistantPermission, AssistantActionSupport>;
      note: string;
      partial: boolean;
    }[];
  }[];
  arbitraryCommand: Record<AssistantPermission, AssistantActionSupport>;
  resourceChanges: AssistantActionSupport;
}

export interface AssistantBootstrap {
  owner?: string;
  authorityGeneration?: number;
  tools?: AssistantToolCapabilities;
  modelUnavailableReason?: string | null;
  search?: {
    available: boolean;
    unitPrice?: string | null;
    currency?: string | null;
    unit?: string | null;
    priceVersion?: string | null;
    reason?: string | null;
  };
  reauthorizationRequired?: boolean;
  runtime: {
    available: boolean;
    reason?: string | null;
    version?: string | null;
  };
  models: AssistantModel[];
  defaultModelId: string | null;
  defaultModelIds?: Partial<Record<AssistantKind, string | null>>;
}

export interface AssistantRestorePreview {
  receipt: string;
  sessionCount: number;
  fileCount: number;
  newRecords: number;
  skippedRecords: number;
  conflicts: string[];
}

export interface AssistantRunRef {
  runId: string;
  operationId: string;
}

export interface AssistantSession {
  id: string;
  kind: AssistantKind;
  title: string;
  modelId: string;
  permissionMode: AssistantPermission;
  webSearch: boolean;
  draft?: string;
  draftAttachmentIds?: string[];
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
  activeRun?: AssistantRunRef | null;
}

export interface AssistantAttachment {
  id: string;
  name: string;
  mimeType: string;
  sizeBytes: number;
  status: "ready" | "processing" | "error";
  reason?: string | null;
  /** A backend-owned asset URL, never a renderer-provided filesystem path. */
  previewUrl?: string | null;
}

export interface AssistantSource {
  id: string;
  title: string;
  url: string;
  excerpt?: string | null;
}

export interface AssistantMessage {
  id: string;
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  createdAt: number;
  reasoning?: string | null;
  status?: "completed" | "streaming" | "stopped" | "error";
  attachments?: AssistantAttachment[];
  sources?: AssistantSource[];
  /** Old branches remain readable when editing or regenerating. */
  superseded?: boolean;
  revisionOf?: string | null;
  nativeEntryId?: string | null;
}

export interface AssistantMemory {
  id: string;
  content: string;
  updatedAt: number;
}

export interface AssistantUsage {
  usageCoverage?: "complete" | "incomplete";
  searchCount?: number;
  searchEstimatedCostUsd?: string | null;
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
  estimatedCostUsd?: string | null;
  billedCostUsd?: string | null;
}
export interface AssistantUsageSummary extends AssistantUsage {
  sessionCount: number;
  runCount: number;
}

export interface AssistantOperation {
  action?: "openTools";
  toolName?: string;
  id: string;
  title: string;
  summary?: string | null;
  status: "running" | "completed" | "failed" | "cancelled" | "unknown";
  output?: string | null;
  command?: string | null;
  exitCode?: number | null;
}

export interface AssistantApproval {
  id: string;
  title: string;
  description: string;
  details?: string | null;
}

export interface AssistantSessionDetail {
  session: AssistantSession;
  messages: AssistantMessage[];
  memories: AssistantMemory[];
  attachments?: AssistantAttachment[];
  usage?: AssistantUsage | null;
  operations?: AssistantOperation[];
  pendingApproval?: AssistantApproval | null;
}

export interface AssistantUpdateRequest {
  sessionId: string;
  expectedOwner?: string;
  expectedGeneration?: number;
  title?: string;
  modelId?: string;
  permissionMode?: AssistantPermission;
  /** Required when explicitly granting full access; backend validates this. */
  fullAccessAcknowledged?: boolean;
  webSearch?: boolean;
  draft?: string;
  draftAttachmentIds?: string[];
  deleted?: boolean;
}

export interface AssistantSendRequest extends AssistantRunRef {
  sessionId: string;
  text: string;
  attachmentIds: string[];
  editMessageId?: string;
  regenerateMessageId?: string;
}

export type AssistantApprovalDecision = "allowOnce" | "allowSession" | "deny";

/** Host-normalized payload. Unknown event kinds are ignored by the renderer. */
export interface AssistantEvent {
  sessionId: string;
  runId: string;
  operationId: string;
  type: string;
  data: {
    delta?: string;
    messageId?: string;
    message?: AssistantMessage | string;
    sources?: AssistantSource[];
    approval?: AssistantApproval;
    operation?: AssistantOperation;
    usage?: AssistantUsage;
    session?: AssistantSession;
    code?: string;
    [key: string]: unknown;
  };
}

export const ASSISTANT_EVENT = "assistant:event";

/** The backend derives account, organization and region; callers never supply owner IDs. */
export const assistantApi = {
  backup: () => invoke<{ path: string | null }>("assistant_backup"),
  previewRestore: () =>
    invoke<AssistantRestorePreview | null>("assistant_preview_restore"),
  restore: (receipt: string) =>
    invoke<{ importedRecords: number }>("assistant_restore", { receipt }),
  usage: () => invoke<AssistantUsageSummary>("assistant_usage"),
  bootstrap: () => invoke<AssistantBootstrap>("assistant_bootstrap"),
  listSessions: (kind: AssistantKind, deleted = false, search = "") =>
    invoke<AssistantSession[]>("assistant_list_sessions", {
      kind,
      deleted,
      search,
    }),
  createSession: (request: {
    kind: AssistantKind;
    modelId?: string;
    permissionMode: AssistantPermission;
    webSearch: boolean;
  }) => invoke<AssistantSession>("assistant_create_session", { request }),
  getSession: (sessionId: string) =>
    invoke<AssistantSessionDetail>("assistant_get_session", { sessionId }),
  updateSession: (request: AssistantUpdateRequest) =>
    invoke<AssistantSession>("assistant_update_session", { request }),
  send: (request: AssistantSendRequest) =>
    invoke<AssistantRunRef>("assistant_send", { request }),
  stop: (sessionId: string, runId: string, operationId: string) =>
    invoke<void>("assistant_stop", { sessionId, runId, operationId }),
  pickAttachments: (sessionId: string) =>
    invoke<AssistantAttachment[]>("assistant_pick_attachments", { sessionId }),
  exportSession: (sessionId: string) =>
    invoke<{ path: string | null }>("assistant_export_session", { sessionId }),
  purgeSession: (sessionId: string) =>
    invoke<void>("assistant_purge_session", { sessionId }),
  saveMemory: (request: { sessionId: string; id?: string; content: string }) =>
    invoke<AssistantMemory>("assistant_save_memory", { request }),
  deleteMemory: (sessionId: string, memoryId: string) =>
    invoke<void>("assistant_delete_memory", { sessionId, memoryId }),
  respondApproval: (request: {
    sessionId: string;
    runId: string;
    operationId: string;
    approvalId: string;
    decision: AssistantApprovalDecision;
  }) => invoke<void>("assistant_respond_approval", { request }),
  subscribe: (callback: (event: AssistantEvent) => void): Promise<UnlistenFn> =>
    listen<AssistantEvent>(ASSISTANT_EVENT, ({ payload }) => callback(payload)),
};
