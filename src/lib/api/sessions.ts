import { invoke } from "@tauri-apps/api/core";
import type { SessionMessage, SessionMeta } from "@/types";
import type { UsageEstimate, UsagePricingCoverage } from "@/types/usageQuery";

export interface SessionCapabilities {
  canReadMessages: boolean;
  canExport: boolean;
  canResume: boolean;
  canTrash: boolean;
  resumeUsesSessionModel: boolean;
  readOnlyReason?: string | null;
  resumeUnavailableReason?: string | null;
}

export type SessionFileKind = "main" | "subagent" | "segment" | "database";

export interface SessionFile {
  path: string;
  sizeBytes: number;
  kind: SessionFileKind;
}

/** One model's share of a session's usage. */
export interface SessionModelUsage {
  model: string;
  requests: number;
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  /** null: the model spent tokens at no known price. */
  costUsd?: number | null;
}

export interface SessionUsage {
  models: SessionModelUsage[];
  inputTokens: number;
  outputTokens: number;
  cacheReadTokens: number;
  cacheWriteTokens: number;
  totalTokens: number;
  costUsd: number;
  unpricedModels: number;
}

/** Proven, thread-local display projection; never a rewritten billing ledger. */
export interface SessionUsageDisplay {
  state: "available" | "unavailable";
  basis: "currentOfoxEstimate";
  scope: "nativeThread";
  reasons: string[];
  estimate: UsageEstimate | null;
  pricingCoverage?: UsagePricingCoverage | null;
  modelEstimates?: Record<string, UsageEstimate>;
  parentSessionId: string | null;
  billingSessionId: string | null;
}

/** Emitted when titles, counts or usage of the listing may have changed. */
export const SESSIONS_INDEX_UPDATED_EVENT = "sessions-index-updated";

export interface ManagedSession extends SessionMeta {
  sessionKey: string;
  revision?: string | null;
  capabilities: SessionCapabilities;
  models?: string[];
  subagentCount?: number;
  /** Bytes of every file of the session (0 for a shared database). */
  sizeBytes?: number;
  files?: SessionFile[];
  usage?: SessionUsage;
  usageDisplay?: SessionUsageDisplay;
}

/** Providers may expose observed models or the usage database's model breakdown. */
export function sessionModelNames(
  session: SessionMeta & {
    models?: string[];
    usage?: Pick<SessionUsage, "models">;
  },
): string[] {
  return [
    ...new Set([
      ...(session.models ?? []),
      ...(session.usage?.models.map((m) => m.model) ?? []),
    ]),
  ];
}

export interface SessionTrashEntry {
  key: string;
  providerId: string;
  sessionId: string;
  title?: string | null;
  projectDir?: string | null;
  deletedAt: number;
  state: "trashed" | "preparing" | "restoring" | "recoveryRequired";
  error?: string | null;
  itemCount: number;
  /** Absent for trash written by older app versions. */
  sizeBytes?: number | null;
}

export interface TrashSessionResult {
  sessionKey: string;
  success: boolean;
  trashKey?: string;
  error?: string;
}

export interface DeleteSessionOptions {
  providerId: string;
  sessionId: string;
  sourcePath: string;
}

export interface DeleteSessionResult extends DeleteSessionOptions {
  success: boolean;
  error?: string;
}

export interface SessionSearchHit {
  sessionKey: string;
  readRevision: string;
  messageOrdinal: number;
  contentHash: string;
  role: SessionMessage["role"];
  name?: string | null;
  snippet: string;
}

export interface SessionSearchProgress {
  operationId: string;
  requestId: string;
  hits: SessionSearchHit[];
  scanned: number;
  total: number;
  changed: number;
  failed: number;
  unsupported: number;
}

export interface SessionSearchResult extends SessionSearchProgress {
  state: "complete" | "limited" | "cancelled";
  limitReasons: string[];
  nextCursor?: string | null;
}

export interface SessionTranscriptResult {
  readRevision: string;
  messages: SessionMessage[];
}

export const sessionsApi = {
  async prepareSearch(request: {
    requestId: string;
    query: string;
    sessionKeys: string[];
    includeProcess: boolean;
  }) {
    return await invoke<{
      operationId: string;
      requestId: string;
      candidateCount: number;
    }>("prepare_session_search", request);
  },

  async runSearch(operationId: string, cursor?: string) {
    return await invoke<SessionSearchResult>("run_session_search", {
      operationId,
      cursor,
    });
  },

  async cancelSearch(operationId: string) {
    return await invoke<boolean>("cancel_session_search", { operationId });
  },

  async getTranscript(
    sessionKey: string,
    expected?: Pick<
      SessionSearchHit,
      "readRevision" | "messageOrdinal" | "contentHash"
    >,
  ) {
    return await invoke<SessionTranscriptResult>("get_session_transcript", {
      sessionKey,
      expectedReadRevision: expected?.readRevision,
      expectedMessageOrdinal: expected?.messageOrdinal,
      expectedContentHash: expected?.contentHash,
    });
  },

  async list(): Promise<ManagedSession[]> {
    return await invoke("list_sessions");
  },

  /** Sync usage now, read changed session files again, return the listing. */
  async refreshIndex(): Promise<ManagedSession[]> {
    return await invoke("refresh_session_index");
  },

  async exportMarkdown(sessionKey: string, destination?: string) {
    return await invoke<{ filename: string; content: string }>(
      "export_session_markdown",
      { sessionKey, destination },
    );
  },

  async resume(sessionKey: string, launch: boolean) {
    return await invoke<{ command: string; launched: boolean }>(
      "resume_session",
      {
        sessionKey,
        launch,
      },
    );
  },

  async trash(items: { sessionKey: string; revision: string }[]) {
    return await invoke<TrashSessionResult[]>("trash_sessions", { items });
  },

  async listTrash() {
    return await invoke<SessionTrashEntry[]>("list_session_trash");
  },

  async restore(key: string) {
    return await invoke<SessionTrashEntry>("restore_session_trash", { key });
  },

  async purge(key: string) {
    return await invoke<void>("purge_session_trash", { key });
  },

  async getMessages(
    providerId: string,
    sourcePath: string,
  ): Promise<SessionMessage[]> {
    // Compatibility for the retired page: resolve a server-issued reference first.
    const records = await sessionsApi.list();
    const record = records.find(
      (session) =>
        session.providerId === providerId && session.sourcePath === sourcePath,
    );
    if (!record?.sessionKey)
      throw new Error("Session is no longer available; refresh the listing");
    return await sessionsApi.getMessagesByKey(record.sessionKey);
  },

  async getMessagesByKey(sessionKey: string): Promise<SessionMessage[]> {
    return await invoke("get_session_messages", { sessionKey });
  },

  async delete(options: DeleteSessionOptions): Promise<boolean> {
    const { providerId, sessionId, sourcePath } = options;
    return await invoke("delete_session", {
      providerId,
      sessionId,
      sourcePath,
    });
  },

  async deleteMany(
    items: DeleteSessionOptions[],
  ): Promise<DeleteSessionResult[]> {
    return await invoke("delete_sessions", { items });
  },

  async launchTerminal(options: {
    command: string;
    cwd?: string | null;
    customConfig?: string | null;
  }): Promise<boolean> {
    const { command, cwd, customConfig } = options;
    return await invoke("launch_session_terminal", {
      command,
      cwd,
      customConfig,
    });
  },
};
