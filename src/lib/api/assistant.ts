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
  /** What the model makes; conversations are offered chat models only. */
  kind?: AssistantModelKind;
  /** The id's prefix (`openai`, `anthropic`, …); empty when it has none. */
  vendor?: string;
  /** `name` without a leading "Vendor: ", for a list grouped by vendor. */
  shortName?: string;
  /** The API a chat model runs with; null for the other kinds. */
  protocol?: AssistantModelProtocol | null;
  /** The catalog lists a `reasoning` parameter. */
  reasoning?: boolean;
  /** The model takes tools; without them a run gets no web search. */
  tools?: boolean;
  /**
   * The thinking levels a conversation may choose, in Pi's order; empty when
   * the model does not reason. No level (the default) sends nothing.
   */
  thinkingLevels?: AssistantThinkingLevel[];
  /** Pi's map from a level to what the provider is sent; null: not offered. */
  thinkingLevelMap?: Partial<Record<AssistantThinkingLevel, string | null>>;
  /** Pi's thinking compat for a Claude model run with Messages. */
  anthropicCompat?: {
    forceAdaptiveThinking?: boolean;
    supportsTemperature?: boolean;
  };
  /** When the model was added, in Unix seconds. */
  created?: number | null;
  /** Image models only. */
  image?: AssistantImageCapabilities;
  /** Video models only. */
  video?: AssistantVideoCapabilities;
}

export type AssistantModelKind =
  | "chat"
  | "image"
  | "video"
  | "speech"
  | "transcription";

/** Pi's thinking levels, lowest first. */
export type AssistantThinkingLevel =
  | "off"
  | "minimal"
  | "low"
  | "medium"
  | "high"
  | "xhigh"
  | "max";

export type AssistantModelProtocol =
  | "openai-completions"
  | "openai-responses"
  | "anthropic-messages";

/** Prices are decimal USD strings; token prices are per million tokens. */
export interface AssistantImageCapabilities {
  generations: boolean;
  edits: boolean;
  n: boolean;
  sizes: string[];
  qualities: string[];
  perImagePrice: string | null;
  tokenPricing: {
    input: string | null;
    inputImage: string | null;
    outputImage: string;
  } | null;
}

export interface AssistantVideoCapabilities {
  modes: string[];
  resolutions: string[];
  defaultResolution: string | null;
  /** Seconds. */
  minDuration: number | null;
  maxDuration: number | null;
  supportsAudio: boolean;
  aspectRatios: string[];
  /** USD per second at the catalog's listed rate. */
  fromPricePerSecond: string | null;
}

/** A model catalog the backend could not read: some models are missing. */
export type AssistantCatalogSource = "v1" | "anthropic";

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
  /** Catalog sources that could not be read; empty or absent when complete. */
  partial?: AssistantCatalogSource[];
  /** The dedicated assistant key is created on the first send, never here. */
  key: { status: "ready" | "missing" };
  /** Left out by a backend without web search: read as unavailable. */
  search?: AssistantSearchStatus;
}

/**
 * Whether runs can search the web: only while the catalog prices the search
 * model. A conversation with `webSearch` on runs without web tools otherwise.
 */
export interface AssistantSearchStatus {
  available: boolean;
  /**
   * USD per billed search call as a decimal string; one search makes one or
   * two, and the search model's tokens come on top. Null when unavailable.
   */
  unitPriceUsd: string | null;
  model: string;
}

/** A page a reply's web searches found: always HTTPS on a public host. */
export interface AssistantSource {
  id: string;
  /** The page's title, or its URL when it had none. */
  title: string;
  url: string;
  /** May be empty. */
  excerpt: string;
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
  /**
   * The chosen thinking level; null is the default (no thinking parameter).
   * A level the model no longer offers is not sent.
   */
  thinkingLevel?: AssistantThinkingLevel | null;
  draft: string;
  draftAttachmentIds: string[];
  createdAt: number;
  updatedAt: number;
  deletedAt: number | null;
  /** The live run only; null once it ended. */
  activeRun: AssistantRunRef | null;
  /** This update committed; only unused disk-copy cleanup needs retrying. */
  cleanupWarning?: string;
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
  /** Readable conversations in the trash, whatever the filter and search. */
  trashCount: number;
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
  /**
   * How long the reply streamed reasoning, in ms: each span from
   * `thinking_start` to its `thinking_end` (or the next text, tool call or
   * end of message). At least 1 whenever reasoning streamed; null when none
   * did.
   */
  reasoningMs?: number | null;
  status?: "completed" | "streaming" | "stopped" | "error";
  /** Stable failure code saved with the reply; older messages may omit it. */
  errorCode?: string | null;
  attachments?: AssistantAttachment[];
  /** The pages the reply's web searches found; `[]` on every other message. */
  sources?: AssistantSource[];
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
  /** Paid web searches in the conversation; counted again once a run is saved. */
  searchCount?: number;
  /**
   * What those searches cost; already part of `estimatedCostUsd`. Null
   * whenever that is, and while a run goes on.
   */
  searchCostUsd?: string | null;
  usageCoverage?: "complete" | "incomplete";
}

export interface AssistantTransferRequest {
  expectedOwner: string;
  expectedGeneration: number;
}

/** What a finished backup holds: `skipped` damaged conversations were left out. */
export interface AssistantBackupReport {
  sessions: number;
  skipped: number;
}

/**
 * `imported` conversations are new here; `skipped` were already here and
 * identical; `conflicts` are here with other content, or in use, and were
 * left as they are. `warning` is a code for the page's error banner.
 */
export interface AssistantRestoreReport {
  imported: number;
  skipped: number;
  conflicts: number;
  warning: string | null;
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
  /**
   * One of the model's `thinkingLevels`, or "default" to clear the choice.
   * Never stops a running reply.
   */
  thinkingLevel?: AssistantThinkingLevel | "default";
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
   * turn's own files are read again). For an edit, an empty list removes
   * every file when `replaceAttachments` is true; older callers keep them.
   */
  attachmentIds: string[];
  replaceAttachments?: boolean;
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
  /**
   * `messageId` is the run's reply. `webSearch` is whether the run got web
   * search: false with the conversation's switch on means search was
   * unavailable when it was sent (or the model takes no tools).
   * `thinkingLevel` is the level the host sent the run, null for the
   * default. It is one the model's `thinkingLevels` lists, which are Pi's
   * own levels for it, so Pi runs it as sent rather than clamping it.
   */
  | {
      type: "run_start";
      data: {
        messageId: string;
        webSearch: boolean;
        thinkingLevel?: AssistantThinkingLevel | null;
      };
    }
  /**
   * The reply began streaming reasoning, as soon as its first reasoning
   * arrived: the reasoning text itself can follow a little later. It lasts
   * until `thinking_end` (a live timer runs from here). Hidden reasoning
   * sends neither.
   */
  | { type: "thinking_start"; data: { messageId: string } }
  /**
   * The reasoning stopped: the model began its text or a tool call, or the
   * message ended. Its last reasoning came first; the text that follows can
   * come a little later.
   */
  | { type: "thinking_end"; data: { messageId: string } }
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
  /**
   * A tool call of the run's reply. `args` holds only what may be shown:
   * `query` (web_search), `url` (web_fetch) or `id` (read_attachment).
   */
  | {
      type: "tool_start";
      data: {
        messageId: string;
        name: string;
        toolCallId: string;
        args: Record<string, string>;
      };
    }
  /**
   * `code` names a failure the model saw and continued from; null on success.
   * A stop or failure may end the run without a call's `tool_end`: `run_end`
   * ends every call still running.
   */
  | {
      type: "tool_end";
      data: {
        messageId: string;
        name: string;
        toolCallId: string;
        success: boolean;
        code: string | null;
      };
    }
  /**
   * All the reply's sources so far, already saved: replaces the list. Sent
   * between a web search's `tool_start` and `tool_end`.
   */
  | {
      type: "sources";
      data: { messageId: string; sources: AssistantSource[] };
    }
  | { type: "compaction_start"; data: Record<string, never> }
  | { type: "compaction_end"; data: { success: boolean; willRetry?: boolean } }
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
  usageSummary: () => invoke<AssistantUsage>("assistant_usage_summary"),
  /** `null` when the save dialog was cancelled. */
  backup: (request: AssistantTransferRequest) =>
    invoke<AssistantBackupReport | null>("assistant_backup", { request }),
  restore: (request: AssistantTransferRequest) =>
    invoke<AssistantRestoreReport | null>("assistant_restore", { request }),
  listSessions: (deleted = false, search = "") =>
    invoke<AssistantSessionList>("assistant_list_sessions", {
      deleted,
      search,
    }),
  createSession: (request: {
    modelId?: string;
    /** The user picked `modelId`: it is remembered even while some models are missing. */
    modelPicked?: boolean;
    webSearch: boolean;
  }) => invoke<AssistantSession>("assistant_create_session", { request }),
  getSession: (sessionId: string) =>
    invoke<AssistantSessionDetail>("assistant_get_session", { sessionId }),
  updateSession: (request: AssistantUpdateRequest) =>
    invoke<AssistantSession>("assistant_update_session", { request }),
  deleteSession: (request: {
    sessionId: string;
    expectedOwner: string;
    expectedGeneration: number;
  }) => invoke<null>("assistant_delete_session", { request }),
  exportMarkdown: (request: {
    sessionId: string;
    expectedOwner: string;
    expectedGeneration: number;
    markdown: string;
  }) => invoke<boolean>("assistant_export_markdown", { request }),
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
 * and `assistant_request_expired`. The renderer also raises
 * `assistant_transfer_busy` when it refuses a backup or restore itself. Codes
 * that mean the same thing to the user share a message. A guard test keeps
 * this list equal to the backend's.
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
  assistant_transfer_busy: "transferBusy",
  assistant_operation_already_submitted: "runChanged",
  assistant_stale_operation: "runChanged",
  assistant_session_not_found: "sessionMissing",
  assistant_session_deleted: "sessionMissing",
  assistant_session_not_trashed: "sessionNotTrashed",
  assistant_delete_pending: "deletePending",
  assistant_backup_invalid: "backupInvalid",
  assistant_backup_too_large: "backupTooLarge",
  assistant_backup_conflict: "backupConflict",
  assistant_backup_failed: "backupFailed",
  assistant_backup_contains_key: "backupContainsKey",
  assistant_backup_owner_mismatch: "backupOwnerMismatch",
  assistant_export_failed: "exportFailed",
  assistant_export_destination_invalid: "exportDestinationInvalid",
  assistant_export_too_large: "exportTooLarge",
  assistant_invalid_identifier: "sessionMissing",
  assistant_invalid_session: "sessionUnreadable",
  assistant_message_not_found: "messageMissing",
  assistant_branch_unavailable: "messageMissing",
  assistant_invalid_message: "invalidMessage",
  assistant_message_too_large: "messageTooLarge",
  assistant_title_too_long: "titleTooLong",
  assistant_invalid_thinking_level: "thinkingLevelUnavailable",
  assistant_attachment_not_found: "attachmentUnavailable",
  assistant_attachment_too_many: "attachmentTooMany",
  assistant_attachment_too_large: "attachmentTooLarge",
  assistant_attachment_total_too_large: "attachmentTotalTooLarge",
  assistant_attachment_unsupported: "attachmentUnsupported",
  assistant_attachment_unreadable: "attachmentUnreadable",
  assistant_attachment_write_failed: "attachmentWriteFailed",
  assistant_attachment_cleanup_failed: "attachmentCleanupFailed",
  assistant_attachment_dialog_failed: "attachmentDialogFailed",
  assistant_vision_model_required: "visionModelRequired",
  assistant_choose_model: "modelRequired",
  assistant_model_unavailable: "modelUnavailable",
  assistant_no_compatible_models: "noModels",
  assistant_catalog_unavailable: "catalogUnavailable",
  assistant_catalog_invalid: "catalogUnavailable",
  assistant_catalog_too_large: "catalogUnavailable",
  assistant_run_timeout: "runTimeout",
  assistant_search_outcome_unknown: "searchOutcomeUnknown",
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
