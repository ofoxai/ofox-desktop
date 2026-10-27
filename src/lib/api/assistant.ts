import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// The renderer's half of the assistant contract. Every command, event and
// error code the backend speaks is typed here; components never call invoke.
// The shapes follow `src-tauri/src/commands/assistant.rs` and
// `src-tauri/src/services/assistant/`.

export interface AssistantModel {
  id: string;
  name: string;
  contextWindow: number;
  maxOutputTokens: number;
  supportsImages: boolean;
  inputModalities: string[];
  supportedEndpoints: string[];
  inputPricePerMillion: string | null;
  outputPricePerMillion: string | null;
  pricing: Record<string, unknown>;
  cost: {
    input: number | null;
    output: number | null;
    cacheRead: number | null;
    cacheWrite: number | null;
  };
  pricingKnown: boolean;
}

export interface AssistantBootstrap {
  /** The signed-in account as the backend keys its data (64 hex). */
  owner: string;
  /** Bumped by every sign-in, sign-out, region change and auth expiry. */
  authorityGeneration: number;
  runtime: {
    available: boolean;
    reason: string | null;
    version: string | null;
  };
  models: AssistantModel[];
  defaultModelId: string | null;
  /** A catalog code when the model list could not be loaded. */
  modelUnavailableReason: string | null;
  /** The dedicated assistant key is created on the first send, never here. */
  key: { status: "ready" | "missing" };
}

export interface AssistantRunRef {
  runId: string;
  operationId: string;
}

export interface AssistantSession {
  id: string;
  title: string;
  modelId: string;
  webSearch: boolean;
  draft: string;
  draftAttachmentIds: string[];
  createdAt: number;
  updatedAt: number;
  deletedAt: number | null;
  /** The live run only; null once it ended. */
  activeRun: AssistantRunRef | null;
}

/** A saved conversation the backend could not read: nothing but its id is known. */
export interface AssistantUnreadableSession {
  id: string;
  /** `assistant_invalid_session` or a storage code. */
  error: string;
}

export interface AssistantSessionList {
  sessions: AssistantSession[];
  unreadable: AssistantUnreadableSession[];
}

/**
 * `ready` once imported; afterwards the outcome of the last run that read the
 * file. `ready` and `partial` were sent; `model_unsupported` needs a model
 * that can read images; the others cannot be read at all.
 */
export type AssistantAttachmentStatus =
  | "ready"
  | "partial"
  | "unreadable"
  | "unsupported"
  | "too_large"
  | "model_unsupported";

/**
 * A file imported into a conversation. In `AssistantSessionDetail.attachments`
 * it carries the last outcome any run reported for it; on a user message the
 * outcome of that turn.
 */
export interface AssistantAttachment {
  id: string;
  name: string;
  mimeType: string;
  sizeBytes: number;
  status: AssistantAttachmentStatus;
  /** The outcome's code, e.g. `assistant_attachment_partial`; null when ready. */
  reason: string | null;
  /** Unused: no previews are shown. */
  previewUrl?: string | null;
  /**
   * The runtime's outcome details (assistant-runtime README "details
   * fields"): `format`, `pageCount`, `pagesWithoutText`, `truncated`,
   * `limit`, `maxBytes`, `reason`, …; null until a run has read the file.
   */
  details: Record<string, unknown> | null;
}

/**
 * The backend always sends every field. Those marked optional are left out
 * only by the renderer's own placeholders (a question being sent, a reply
 * being streamed).
 */
export interface AssistantMessage {
  id: string;
  role: "user" | "assistant";
  content: string;
  createdAt: number;
  reasoning?: string | null;
  status?: "completed" | "streaming" | "stopped" | "error";
  attachments?: AssistantAttachment[];
  sources?: unknown[];
  /** Old branches remain readable when editing or regenerating. */
  superseded?: boolean;
  revisionOf?: string | null;
  nativeEntryId?: string | null;
  runId?: string | null;
}

/**
 * A conversation's token and cost totals; `{}` before its first request.
 * During a run they are running totals and the cost is null until the run is
 * saved. `usageCoverage: "incomplete"` means some request may have been billed
 * without a known cost, for good: the cost is then unknown, never zero.
 */
export interface AssistantUsage {
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
  /** A decimal string, or null when unknown. */
  estimatedCostUsd?: string | null;
  billedCostUsd?: null;
  modelRequests?: number;
  usageCoverage?: "incomplete";
}

export interface AssistantSessionDetail {
  session: AssistantSession;
  messages: AssistantMessage[];
  attachments: AssistantAttachment[];
  usage: AssistantUsage;
}

export interface AssistantUpdateRequest {
  sessionId: string;
  expectedOwner?: string;
  expectedGeneration?: number;
  title?: string;
  /** Changing the model stops a running reply. */
  modelId?: string;
  webSearch?: boolean;
  draft?: string;
  draftAttachmentIds?: string[];
  deleted?: boolean;
}

export interface AssistantSendRequest extends AssistantRunRef {
  sessionId: string;
  /** May be empty when at least one file is attached. */
  text: string;
  /**
   * The files sent with a new turn, in order. Ignored by a regenerate (its
   * turn's own files are read again); for an edit `[]` keeps the edited
   * message's files and a non-empty list replaces them.
   */
  attachmentIds: string[];
  editMessageId?: string;
  regenerateMessageId?: string;
}

/**
 * What the host publishes for a run, in order: `run_start` (before `send`
 * returns), deltas, usage and compaction while it runs, then, once the turn
 * is saved, `message`, `session`, `error` (only for a failure that was not a
 * stop) and `run_end` last.
 */
export type AssistantRunEvent =
  | { type: "run_start"; data: { messageId: string } }
  | {
      type: "text_delta" | "thinking_delta";
      data: { delta: string; messageId: string };
    }
  /**
   * One per file sent with the turn, after it was read and before the model
   * request. Saved on the file and on the run's user message first.
   */
  | {
      type: "attachment";
      data: {
        id: string;
        status: AssistantAttachmentStatus;
        code: string | null;
        details: Record<string, unknown> | null;
      };
    }
  /** The conversation's running totals, not this run's. */
  | { type: "usage"; data: { usage: AssistantUsage } }
  | { type: "compaction_start"; data: Record<string, never> }
  | { type: "compaction_end"; data: { success: boolean } }
  /** The run's saved assistant message. */
  | { type: "message"; data: { message: AssistantMessage } }
  /** The saved session; its `activeRun` is null. */
  | { type: "session"; data: { session: AssistantSession } }
  /** `message` is scrubbed provider text: never shown. */
  | { type: "error"; data: { code: string; message?: string } }
  | {
      type: "run_end";
      data: { stopped: boolean; status: "completed" | "stopped" | "error" };
    };

export type AssistantEvent = AssistantRunRef & {
  sessionId: string;
} & AssistantRunEvent;

export const ASSISTANT_EVENT = "assistant:event";

/** The backend derives account, organization and region; callers never supply owner IDs. */
export const assistantApi = {
  bootstrap: () => invoke<AssistantBootstrap>("assistant_bootstrap"),
  listSessions: (deleted = false, search = "") =>
    invoke<AssistantSessionList>("assistant_list_sessions", {
      deleted,
      search,
    }),
  createSession: (request: { modelId?: string; webSearch: boolean }) =>
    invoke<AssistantSession>("assistant_create_session", { request }),
  getSession: (sessionId: string) =>
    invoke<AssistantSessionDetail>("assistant_get_session", { sessionId }),
  updateSession: (request: AssistantUpdateRequest) =>
    invoke<AssistantSession>("assistant_update_session", { request }),
  /**
   * Opens the native file dialog and imports the chosen files into the
   * conversation, all or nothing. Returns them in pick order; `[]` when the
   * dialog was cancelled.
   */
  pickAttachments: (sessionId: string) =>
    invoke<AssistantAttachment[]>("assistant_pick_attachments", { sessionId }),
  send: (request: AssistantSendRequest) =>
    invoke<AssistantRunRef>("assistant_send", { request }),
  stop: (sessionId: string, runId: string, operationId: string) =>
    invoke<null>("assistant_stop", { sessionId, runId, operationId }),
  subscribe: (callback: (event: AssistantEvent) => void): Promise<UnlistenFn> =>
    listen<AssistantEvent>(ASSISTANT_EVENT, ({ payload }) => callback(payload)),
  /**
   * The backend's sign-in state events; either one ends the identity the
   * workspace was showing. Names are literal (as in useModelSelection) so a
   * test that mocks the auth API module cannot turn them into `undefined`.
   * Both or neither: when one cannot be registered the other is released and
   * the failure is thrown.
   */
  onAuthChange: async (
    callback: (state: "expired" | "restored") => void,
  ): Promise<UnlistenFn> => {
    const results = await Promise.allSettled([
      listen("ofox-auth-expired", () => callback("expired")),
      listen("ofox-auth-restored", () => callback("restored")),
    ]);
    const offs = results.flatMap((result) =>
      result.status === "fulfilled" ? [result.value] : [],
    );
    const failed = results.find(
      (result): result is PromiseRejectedResult => result.status === "rejected",
    );
    if (failed) {
      offs.forEach((off) => off());
      throw failed.reason;
    }
    return () => offs.forEach((off) => off());
  },
};

/**
 * Every error code the backend can send (a command rejection, a list row's
 * `error`, a bootstrap reason or a run's `error` event), mapped to its message
 * under `assistant.errors.*`, plus the renderer's own `assistant_session_loading`
 * and `assistant_request_expired`. Codes that mean the same thing to the user
 * share a message. A guard test keeps this list equal to the backend's.
 */
export const ASSISTANT_ERROR_MESSAGES: Record<string, string> = {
  assistant_sign_in_required: "signInRequired",
  assistant_identity_unavailable: "signInRequired",
  assistant_signing_out: "signingOut",
  assistant_shutting_down: "shuttingDown",
  assistant_identity_changed: "identityChanged",
  assistant_request_expired: "identityChanged",
  assistant_unknown_apex: "unknownApex",
  assistant_key_unavailable: "keyUnavailable",
  assistant_key_storage_unavailable: "keyStorageUnavailable",
  assistant_key_rejected: "keyRejected",
  assistant_insufficient_balance: "insufficientBalance",
  assistant_rate_limited: "rateLimited",
  assistant_network_error: "networkError",
  assistant_model_request_failed: "modelRequestFailed",
  assistant_session_loading: "sessionLoading",
  assistant_session_busy: "sessionBusy",
  assistant_operation_already_submitted: "runChanged",
  assistant_stale_operation: "runChanged",
  assistant_session_not_found: "sessionMissing",
  assistant_session_deleted: "sessionMissing",
  assistant_invalid_identifier: "sessionMissing",
  assistant_invalid_session: "sessionUnreadable",
  assistant_message_not_found: "messageMissing",
  assistant_branch_unavailable: "messageMissing",
  assistant_invalid_message: "invalidMessage",
  assistant_message_too_large: "messageTooLarge",
  assistant_title_too_long: "titleTooLong",
  assistant_attachment_not_found: "attachmentUnavailable",
  assistant_attachment_too_many: "attachmentTooMany",
  assistant_attachment_too_large: "attachmentTooLarge",
  assistant_attachment_total_too_large: "attachmentTotalTooLarge",
  assistant_attachment_unsupported: "attachmentUnsupported",
  assistant_attachment_unreadable: "attachmentUnreadable",
  assistant_attachment_write_failed: "attachmentWriteFailed",
  assistant_attachment_dialog_failed: "attachmentDialogFailed",
  assistant_vision_model_required: "visionModelRequired",
  assistant_choose_model: "modelRequired",
  assistant_model_unavailable: "modelUnavailable",
  assistant_no_compatible_models: "noModels",
  assistant_catalog_unavailable: "catalogUnavailable",
  assistant_catalog_invalid: "catalogUnavailable",
  assistant_catalog_too_large: "catalogUnavailable",
  assistant_run_timeout: "runTimeout",
  assistant_cancelled: "stopped",
  assistant_runtime_unavailable: "runtimeMissing",
  assistant_runtime_failed: "runtime",
  assistant_runtime_exited: "runtime",
  assistant_runtime_stdio: "runtime",
  assistant_runtime_protocol: "runtime",
  assistant_runtime_protocol_limit: "runtime",
  assistant_runtime_identity_mismatch: "runtime",
  assistant_process_start_failed: "runtime",
  assistant_storage_unavailable: "storage",
  assistant_invalid_storage_path: "storage",
  assistant_invalid_preferences: "storage",
  assistant_state_unavailable: "internal",
  assistant_native_history_missing: "historyMissing",
  assistant_native_run_missing: "historyMissing",
  assistant_native_unknown_run: "historyMissing",
  assistant_history_recovery_failed: "historyRecovery",
  assistant_history_recovery_timeout: "historyRecovery",
};

/**
 * The code of a command rejection or thrown Error (the backend rejects with
 * the bare code), or of an `error` event's data. An event's `message` is
 * provider text and never decides anything.
 */
export function assistantErrorCode(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object") {
    const { code } = error as { code?: unknown };
    if (typeof code === "string") return code;
  }
  return "";
}

/**
 * The i18n key for an assistant failure. Unknown codes and free text get the
 * generic message, so no raw code or provider text reaches the screen.
 */
export function assistantErrorKey(error: unknown): string {
  const code = assistantErrorCode(error).trim();
  const known = Object.prototype.hasOwnProperty.call(
    ASSISTANT_ERROR_MESSAGES,
    code,
  );
  return `assistant.errors.${known ? ASSISTANT_ERROR_MESSAGES[code] : "generic"}`;
}
