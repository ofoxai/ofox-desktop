import { invoke } from "@tauri-apps/api/core";
import type { SessionMessage, SessionMeta } from "@/types";

export interface SessionCapabilities {
  canReadMessages: boolean;
  canExport: boolean;
  canResume: boolean;
  canTrash: boolean;
  resumeUsesSessionModel: boolean;
  readOnlyReason?: string | null;
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

export const sessionsApi = {
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
