import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

// The renderer's half of the assistant contract. Every command, event and
// error code the backend speaks is typed here; components never call invoke.

export interface AssistantModel {
  id: string;
  name: string;
  contextWindow?: number | null;
  maxOutputTokens?: number | null;
  supportsImages?: boolean;
  inputPricePerMillion?: string | null;
  outputPricePerMillion?: string | null;
}

export interface AssistantBootstrap {
  owner?: string;
  authorityGeneration?: number;
  runtime: {
    available: boolean;
    reason?: string | null;
    version?: string | null;
  };
  models: AssistantModel[];
  defaultModelId: string | null;
  modelUnavailableReason?: string | null;
  /** The dedicated assistant key is created on the first send, never here. */
  key?: { status: "ready" | "missing" };
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
  draft?: string;
  draftAttachmentIds?: string[];
  createdAt: number;
  updatedAt: number;
  deletedAt?: number | null;
  activeRun?: AssistantRunRef | null;
}

/** A saved conversation the backend could not read: nothing but its id is known. */
export interface AssistantUnreadableSession {
  id: string;
  code: string;
}

export interface AssistantSessionList {
  sessions: AssistantSession[];
  unreadable: AssistantUnreadableSession[];
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

export interface AssistantUsage {
  usageCoverage?: "complete" | "incomplete";
  inputTokens?: number;
  outputTokens?: number;
  cacheReadTokens?: number;
  cacheWriteTokens?: number;
  estimatedCostUsd?: string | null;
  billedCostUsd?: string | null;
}

export interface AssistantSessionDetail {
  session: AssistantSession;
  messages: AssistantMessage[];
  attachments?: AssistantAttachment[];
  usage?: AssistantUsage | null;
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
  text: string;
  attachmentIds: string[];
  editMessageId?: string;
  regenerateMessageId?: string;
}

/** Host-normalized payload. Unknown event kinds are ignored by the renderer. */
export interface AssistantEvent {
  sessionId: string;
  runId: string;
  operationId: string;
  /** text_delta, thinking_delta, sources, usage, error, run_end, … */
  type: string;
  data: {
    delta?: string;
    messageId?: string;
    message?: AssistantMessage | string;
    sources?: AssistantSource[];
    usage?: AssistantUsage;
    session?: AssistantSession;
    code?: string;
    [key: string]: unknown;
  };
}

export const ASSISTANT_EVENT = "assistant:event";

interface RawSessionList {
  sessions?: AssistantSession[];
  unreadable?: { id: string; code?: string; error?: string }[];
}

function sessionList(raw: RawSessionList): AssistantSessionList {
  return {
    sessions: raw.sessions ?? [],
    unreadable: (raw.unreadable ?? []).map((row) => ({
      id: row.id,
      code: row.code ?? row.error ?? "",
    })),
  };
}

/** The backend derives account, organization and region; callers never supply owner IDs. */
export const assistantApi = {
  bootstrap: () => invoke<AssistantBootstrap>("assistant_bootstrap"),
  listSessions: async (deleted = false, search = "") =>
    sessionList(
      await invoke<RawSessionList>("assistant_list_sessions", {
        deleted,
        search,
      }),
    ),
  createSession: (request: { modelId?: string; webSearch: boolean }) =>
    invoke<AssistantSession>("assistant_create_session", { request }),
  getSession: (sessionId: string) =>
    invoke<AssistantSessionDetail>("assistant_get_session", { sessionId }),
  updateSession: (request: AssistantUpdateRequest) =>
    invoke<AssistantSession>("assistant_update_session", { request }),
  send: (request: AssistantSendRequest) =>
    invoke<AssistantRunRef>("assistant_send", { request }),
  stop: (sessionId: string, runId: string, operationId: string) =>
    invoke<void>("assistant_stop", { sessionId, runId, operationId }),
  subscribe: (callback: (event: AssistantEvent) => void): Promise<UnlistenFn> =>
    listen<AssistantEvent>(ASSISTANT_EVENT, ({ payload }) => callback(payload)),
};

/**
 * Every error code the assistant can surface, mapped to its message under
 * `assistant.errors.*`. Codes that mean the same thing to the user share a
 * message. The renderer's own codes (`assistant_session_loading`,
 * `assistant_request_expired`) and the sidecar's `runtime_error` are here too.
 */
export const ASSISTANT_ERROR_MESSAGES: Record<string, string> = {
  assistant_sign_in_required: "signInRequired",
  assistant_identity_unavailable: "signInRequired",
  assistant_reauthorization_required: "signInRequired",
  assistant_key_unavailable: "keyUnavailable",
  assistant_signing_out: "signingOut",
  assistant_shutting_down: "shuttingDown",
  assistant_identity_changed: "identityChanged",
  assistant_request_expired: "identityChanged",
  assistant_unknown_apex: "unknownApex",
  assistant_session_loading: "sessionLoading",
  assistant_session_busy: "sessionBusy",
  assistant_operation_already_submitted: "sessionBusy",
  assistant_stale_operation: "sessionBusy",
  assistant_session_not_found: "sessionMissing",
  assistant_session_deleted: "sessionMissing",
  assistant_invalid_identifier: "sessionMissing",
  assistant_invalid_session: "sessionUnreadable",
  assistant_message_not_found: "messageMissing",
  assistant_branch_unavailable: "messageMissing",
  assistant_revision_unavailable: "messageMissing",
  assistant_invalid_message: "invalidMessage",
  assistant_message_too_large: "messageTooLarge",
  assistant_title_too_long: "titleTooLong",
  assistant_choose_model: "modelRequired",
  assistant_model_required: "modelRequired",
  assistant_model_unavailable: "modelUnavailable",
  assistant_model_not_in_catalog: "modelUnavailable",
  assistant_no_compatible_models: "modelUnavailable",
  assistant_catalog_unavailable: "catalogUnavailable",
  assistant_catalog_invalid: "catalogUnavailable",
  assistant_catalog_too_large: "catalogUnavailable",
  assistant_vision_model_required: "visionModelRequired",
  assistant_model_request_failed: "modelRequestFailed",
  runtime_error: "modelRequestFailed",
  assistant_run_timeout: "runTimeout",
  assistant_cancelled: "stopped",
  assistant_runtime_unavailable: "runtime",
  assistant_runtime_failed: "runtime",
  assistant_runtime_exited: "runtime",
  assistant_runtime_stdio: "runtime",
  assistant_runtime_protocol: "runtime",
  assistant_runtime_protocol_limit: "runtime",
  assistant_runtime_identity_mismatch: "runtime",
  assistant_process_start_failed: "runtime",
  assistant_process_stdio: "runtime",
  assistant_process_timeout: "runtime",
  assistant_process_read_failed: "runtime",
  assistant_process_wait_failed: "runtime",
  assistant_process_drain_timeout: "runtime",
  assistant_storage_unavailable: "storage",
  assistant_state_unavailable: "storage",
  assistant_invalid_storage_path: "storage",
  assistant_invalid_preferences: "storage",
  assistant_native_history_missing: "historyMissing",
  assistant_native_run_missing: "historyMissing",
  assistant_native_unknown_run: "historyMissing",
  assistant_history_recovery_failed: "historyMissing",
  assistant_history_recovery_timeout: "historyMissing",
};

/** A wrapper names the stage that failed; a code inside it says why. */
const WRAPPER_CODES = new Set(["assistant_model_request_failed"]);

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  if (error && typeof error === "object") {
    const { code, message } = error as { code?: unknown; message?: unknown };
    return [code, message]
      .filter((part): part is string => typeof part === "string")
      .join(" ");
  }
  return "";
}

/**
 * The i18n key for an assistant failure: a command rejection, a thrown Error,
 * or an `error` event's data. Unknown codes and free text get the generic
 * message, so no raw code or provider text reaches the screen.
 */
export function assistantErrorKey(error: unknown): string {
  const known = (
    errorText(error).match(/[a-z][a-z0-9]*(?:_[a-z0-9]+)+/g) ?? []
  ).filter((code) =>
    Object.prototype.hasOwnProperty.call(ASSISTANT_ERROR_MESSAGES, code),
  );
  const code = known.find((item) => !WRAPPER_CODES.has(item)) ?? known[0];
  return `assistant.errors.${code ? ASSISTANT_ERROR_MESSAGES[code] : "generic"}`;
}
