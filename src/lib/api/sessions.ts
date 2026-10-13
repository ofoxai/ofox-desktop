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

export interface ManagedSession extends SessionMeta {
  sessionKey: string;
  revision?: string | null;
  capabilities: SessionCapabilities;
  models?: string[];
  subagentCount?: number;
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
