import { isWindows } from "@/lib/platform";
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
  /** What the model makes; conversations offer chat and supported images. */
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
  /** Dedicated audio tools; never a conversation model. */
  audio?: AssistantAudioCapabilities;
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

/** Audio rates are USD decimals; token rates are per million tokens. */
export interface AssistantAudioCapabilities {
  family:
    | "gemini-tts"
    | "elevenlabs-tts"
    | "gpt-transcription"
    | "elevenlabs-transcription";
  responseFormats: string[];
  defaultFormat: string;
  voiceKind: "name" | "id" | null;
  /** Documented examples, not an exhaustive provider voice list. */
  suggestedVoices: string[];
  instructions: boolean;
  prompt: boolean;
  language: boolean;
  wordTimestamps: boolean;
  pricing: {
    unit: "tokens" | "seconds" | "characters";
    inputAudioPerSecond: string | null;
    inputTextPerCharacter: string | null;
    inputTextPerMillion: string | null;
    inputAudioPerMillion: string | null;
    outputTextPerMillion: string | null;
    outputAudioPerMillion: string | null;
  };
}

/** Opaque temporary input; audio bytes and source paths stay in Rust. */
export interface AssistantAudioInput {
  inputId: string;
  name: string;
  sizeBytes: number;
  mimeType: "audio/wav" | "audio/mpeg";
  durationMs: number;
  sampleRate: number;
  channels: number;
}

export interface AssistantAudioResult {
  operationId: string;
  runId: string;
  kind: "transcription" | "speech";
  model: string;
  apex?: string | null;
  params: {
    language?: string | null;
    inputName?: string;
    [key: string]: unknown;
  };
  outputs?: string[];
  inputIds?: string[];
  status: "not_sent" | "submitting" | "completed" | "rejected" | "unknown";
  cost: string | null;
  submittedAt?: number | null;
  audio: {
    pricing: AssistantAudioCapabilities["pricing"];
    estimate?: string | null;
    output?: {
      mimeType: string;
      sizeBytes: number;
      durationMs: number;
      sampleRate: number;
      channels: number;
    } | null;
    input: Omit<AssistantAudioInput, "inputId" | "name" | "sizeBytes"> | null;
    text: string | null;
    usage: Record<string, unknown> | null;
    requestId: string | null;
    requestIdSource: string | null;
    errorCode: string | null;
    originalError?: string | null;
  };
}
export interface AssistantTranscribeRequest
  extends AssistantPasteTarget,
    AssistantRunRef {
  inputId: string;
  modelId: string;
  keepOriginal?: boolean;
  language?: "zh" | "en" | "ja";
}

export interface AssistantSpeechRequest
  extends AssistantPasteTarget,
    AssistantRunRef {
  modelId: string;
  input: string;
  voice: string;
  responseFormat: string;
  language?: "zh" | "en" | "ja";
  instructions?: string;
  sourceMessageId?: string;
}

/** The most characters the custom instructions may have. */
export const MAX_ASSISTANT_INSTRUCTIONS = 4000;

/** The assistant settings of the signed-in account. */
export interface AssistantPreferences {
  /**
   * Standing instructions every reply follows, from the next message on;
   * empty for none. At most `MAX_ASSISTANT_INSTRUCTIONS` characters.
   */
  instructions: string;
  /** A conversation's first answer gets it a generated title. */
  autoTitle: boolean;
  /** Tell the user when a reply they are not looking at finishes. */
  notifyOnFinish: boolean;
}

/** What an account that never saved its settings has. */
export const DEFAULT_ASSISTANT_PREFERENCES: AssistantPreferences = {
  instructions: "",
  autoTitle: true,
  notifyOnFinish: true,
};

/** A model catalog the backend could not read: some models are missing. */
export type AssistantCatalogSource = "v1" | "anthropic";

export type AssistantSpeechPreferences = Record<
  string,
  { voice?: string; format?: string }
>;

export interface AssistantBootstrap {
  catalogRevision?: number;
  catalogRefreshing?: boolean;
  catalogRefreshFailed?: AssistantCatalogSource[];
  speechPreferences?: AssistantSpeechPreferences;
  rememberedModels?: Partial<Record<AssistantModelKind, string | null>>;
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
  /** Dedicated tool models, separate from the conversation model picker. */
  audioModels?: AssistantModel[];
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

export type AssistantCatalogSnapshot = Pick<
  AssistantBootstrap,
  | "models"
  | "audioModels"
  | "defaultModelId"
  | "modelUnavailableReason"
  | "partial"
  | "search"
> & {
  catalogRevision: number;
  catalogRefreshing: boolean;
  catalogRefreshFailed: AssistantCatalogSource[];
};

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

export interface AssistantContextUsage {
  modelId: string;
  tokens: number | null;
  contextWindow: number;
}

export interface AssistantSession {
  audioOnly?: boolean;
  contextUsage?: AssistantContextUsage | null;
  /** Current runtime state; never restored from disk. */
  compacting?: boolean;
  id: string;
  title: string;
  /**
   * Who named the conversation: its first message (`auto`; a generated title
   * may replace it) or the user (`user`; never replaced). Any rename is the
   * user's.
   */
  titleSource: AssistantTitleSource;
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
  /** When it was pinned to the top of the list; null when it is not. */
  pinnedAt: number | null;
  /** The live run only; null once it ended. */
  activeRun: AssistantRunRef | null;
  /** This update committed; only unused disk-copy cleanup needs retrying. */
  cleanupWarning?: string;
}

export type AssistantTitleSource = "auto" | "user";

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
  /**
   * For a search: the first active message each listed conversation matches
   * (session id to message id). A conversation found by its title alone, or
   * only through a replaced message, has no entry; `{}` without a search.
   */
  matches: Record<string, string>;
}

/** What emptying the trash did. */
export interface AssistantEmptyTrashReport {
  /** Conversations removed for good. */
  deleted: number;
  /**
   * Conversations not fully removed: one still replying stays in the trash;
   * one whose files could not all be deleted is hidden and cleaned up at the
   * next start. A conversation restored or removed meanwhile is not counted.
   */
  failed: number;
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
  | "model_unsupported"
  | "missing";

/**
 * A file imported into a conversation. In `AssistantSessionDetail.attachments`
 * it carries the last outcome any run reported for it; on a user message the
 * outcome of that turn.
 */
export interface AssistantAttachment {
  origin?: "generated";
  sourceMessageId?: string;
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
  media?: AssistantMedia;
  id: string;
  role: "user" | "assistant" | "media";
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
  status?:
    | "completed"
    | "streaming"
    | "stopped"
    | "error"
    | "queued"
    | "in_progress"
    | "downloading"
    | "paused"
    | "failed"
    | "cancelled"
    | "expired";
  /** Stable failure code saved with the reply; older messages may omit it. */
  errorCode?: string | null;
  attachments?: AssistantAttachment[];
  /** The pages the reply's web searches found; `[]` on every other message. */
  sources?: AssistantSource[];
  /**
   * Not on the version of the conversation shown: an edited, regenerated or
   * retried turn, or another version after a switch. Shown messages are those
   * without it.
   */
  superseded?: boolean;
  revisionOf?: string | null;
  nativeEntryId?: string | null;
  runId?: string | null;
  /** The model a reply was asked of; null on a user turn and on older replies. */
  modelId?: string | null;
  /**
   * What a reply's run used and cost, counted like the conversation's usage
   * but without its generated title. The replies' tokens plus the title's
   * add up to the conversation's, their costs nearly so (float sums); usage
   * records of older histories that name no run count only in the
   * conversation's. Null on a user turn and until the run is counted.
   */
  usage?: AssistantUsage | null;
  /** When a reply's run ended (ms), however it ended. */
  completedAt?: number | null;
  /**
   * For a user turn the model's history holds: the message it follows, null
   * when it starts the conversation. User turns with the same `branchFrom`
   * are versions of one turn (`assistantApi.switchVersion` shows another).
   * Left out when not known: not such a turn, or saved before versions
   * existed — the one field the backend may omit.
   */
  branchFrom?: string | null;
  /**
   * The model's history entry a turn was sent after (internal): a turn the
   * history never got shows with that entry's version. Null on older
   * messages and on a retried turn.
   */
  anchorEntryId?: string | null;
}

/**
 * A conversation's token and cost totals; `{}` before its first request.
 * During a run they are running totals and the cost is null until the run is
 * saved. `usageCoverage: "incomplete"` means some request may have been billed
 * without a known cost, for good: the cost is then unknown, never zero.
 */
export interface AssistantRequestRecord {
  kind: string;
  modelId?: string | null;
  apex?: string | null;
  status: string;
  httpStatus?: number | null;
  requestId?: string | null;
  requestIdSource?: string | null;
  usage?: unknown;
  priceSnapshot?: unknown;
  estimatedCostUsd?: string | null;
  reportedCostUsd?: string | null;
  billedCostUsd?: null;
  jobId?: string | null;
}

export interface AssistantUsage {
  requests?: AssistantRequestRecord[];
  mediaCostUsd?: string | null;
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
  excludeVideos?: boolean;
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
  audioOperations?: AssistantAudioResult[];
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
  /**
   * Pin to, or unpin from, the top of the list. A pin and a draft change do
   * not change `updatedAt`.
   */
  pinned?: boolean;
}

export interface AssistantSendRequest extends AssistantRunRef {
  /** Frozen before admission; the host rejects work left by an old account or region. */
  expectedOwner?: string;
  expectedGeneration?: number;
  video?: AssistantVideoRequest;
  quoteId?: string;
  media?: AssistantImageRequest;
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
  /**
   * A failed or stopped reply to ask again for, with this send's own text
   * and files; that turn becomes superseded. When the model's history holds
   * its question it is asked again from before it (like an edit), otherwise
   * from the end of the conversation. One of edit, regenerate and retry at
   * most.
   */
  retryMessageId?: string;
}

export interface AssistantImageRequest {
  size?: string | null;
  quality?: string | null;
  n?: number | null;
  inputAttachmentId?: string | null;
}

export interface AssistantVideoRequest {
  duration: number;
  resolution: string;
  aspectRatio?: string | null;
  generateAudio?: boolean;
  firstFrameId?: string | null;
  lastFrameId?: string | null;
}
export interface AssistantMediaQuote {
  id: string;
  estimatedCostUsd: string | null;
  expiresAt: number;
}
export interface AssistantVideoActionRequest {
  sessionId: string;
  messageId: string;
  expectedOwner: string;
  expectedGeneration: number;
}

export type AssistantMediaStatus =
  | "submitting"
  | "queued"
  | "in_progress"
  | "downloading"
  | "paused"
  | "query_expired"
  | "download_failed"
  | "completed"
  | "error"
  | "failed"
  | "cancelled"
  | "expired"
  | "unknown";
interface AssistantMediaBase {
  requestAttempts?: {
    httpStatus: number;
    requestId: string | null;
    requestIdSource: string | null;
  }[];
  responseUsage?: unknown;
  priceSnapshot?: unknown;
  model: string;
  inputIds: string[];
  status: AssistantMediaStatus;
  outputs: string[];
  cost: string | null;
  jobId?: string | null;
  apex?: string | null;
  submittedAt?: number | null;
  pollStartedAt?: number | null;
}
export type AssistantMedia = AssistantMediaBase &
  (
    | { kind: "image"; params: AssistantImageRequest }
    | { kind: "video"; params: AssistantVideoRequest }
  );

/**
 * Whether a file drag is over the window. A drop ends the drag: the backend
 * announces `"leave"` first, then the `assistant:drop` event, and no other
 * leave follows. Only the backend knows the dragged paths; the renderer
 * never sees one.
 */
export type AssistantDragState = "enter" | "leave";

/**
 * Files dropped on the window: a single-use token for `importDrop` (valid for
 * 60 seconds, for the account and authority generation of the drop, and
 * replaced by the next drop) and how many files it stands for.
 */
export interface AssistantDrop {
  token: string;
  count: number;
}

export interface AssistantImportDropRequest {
  sessionId: string;
  /** The `token` of the `assistant:drop` event. */
  token: string;
  expectedOwner: string;
  expectedGeneration: number;
}

/** The conversation and authority a pasted file is imported under. */
export interface AssistantPasteTarget {
  sessionId: string;
  expectedOwner: string;
  expectedGeneration: number;
}

export interface AssistantSteerRequest extends AssistantRunRef {
  sessionId: string;
  id: string;
  text: string;
  expectedOwner: string;
  expectedGeneration: number;
}

export interface AssistantSwitchVersionRequest {
  sessionId: string;
  /** The user turn to show: one with a `nativeEntryId`. */
  messageId: string;
  expectedOwner: string;
  expectedGeneration: number;
}

/**
 * What the host publishes for a run, in order: `run_start` (before `send`
 * returns), deltas, usage and compaction while it runs, then, once the turn
 * is saved, `message`, `session`, `error` (only for a failure that was not a
 * stop) and `run_end` last.
 */
export type AssistantRunEvent =
  | {
      type: "catalog_state";
      data: {
        owner: string;
        generation: number;
        catalog: AssistantCatalogSnapshot;
      };
    }
  | {
      type: "audio_state";
      data: {
        owner: string;
        generation: number;
        running: boolean;
        updatedAt: number;
        result: AssistantAudioResult;
        usage: AssistantUsage;
      };
    }
  | {
      type: "steer_applied";
      data: { messageId: string; messages: AssistantMessage[] };
    }
  | {
      type: "media_progress";
      data: {
        owner: string;
        generation: number;
        apex: string;
        messageId: string;
        status: AssistantMediaStatus;
        message: AssistantMessage;
        usage: AssistantUsage;
        elapsedMs: number | null;
        updatedAt: number;
      };
    }
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
        compactOnly?: boolean;
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
  /**
   * The saved session; its `activeRun` is null. A generated title's event
   * also carries the conversation's usage, which the title's request added to.
   */
  | {
      type: "session";
      data: { session: AssistantSession; usage?: AssistantUsage };
    }
  /** `message` is scrubbed provider text: never shown. */
  | { type: "error"; data: { code: string; message?: string } }
  | {
      type: "run_end";
      data: {
        stopped: boolean;
        status: "completed" | "stopped" | "error" | "queued";
      };
    };

export type AssistantEvent = AssistantRunRef & {
  sessionId: string;
} & AssistantRunEvent;

/** The largest file the backend imports (its MAX_FILE_BYTES). */
const MAX_PASTED_FILE_BYTES = 25 * 1024 * 1024;

/** A name as a header value; one that can't be encoded is sent as none. */
function headerFileName(name: string) {
  try {
    return encodeURIComponent(name);
  } catch {
    // A lone surrogate (possible in a Windows name): the backend gives the
    // file a name of its own.
    return "";
  }
}

export const ASSISTANT_EVENT = "assistant:event";
/** A file drag entered or left the window (`AssistantDragState`). */
export const ASSISTANT_DRAG_EVENT = "assistant:drag";
/** Files were dropped on the window (`AssistantDrop`). */
export const ASSISTANT_DROP_EVENT = "assistant:drop";

/** The backend derives account, organization and region; callers never supply owner IDs. */
export const assistantApi = {
  bootstrap: (refreshCatalog = false) =>
    refreshCatalog
      ? invoke<AssistantBootstrap>("assistant_bootstrap", {
          refreshCatalog: true,
        })
      : invoke<AssistantBootstrap>("assistant_bootstrap"),
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
  /** The account's settings; the defaults while its saved ones are damaged. */
  getPreferences: () =>
    invoke<AssistantPreferences>("assistant_get_preferences"),
  /**
   * Saves every setting for the account and authority the page shows and
   * returns them as saved: a key the app handed out is replaced in the
   * instructions. Refused with `assistant_instructions_too_long` over the
   * limit.
   */
  setPreferences: (
    request: AssistantPreferences & {
      expectedOwner?: string;
      expectedGeneration?: number;
    },
  ) => invoke<AssistantPreferences>("assistant_set_preferences", { request }),
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
  /**
   * Permanently deletes every trashed conversation that is not replying. Name
   * the account and authority generation the confirmation showed.
   */
  emptyTrash: (request: {
    expectedOwner: string;
    expectedGeneration: number;
  }) => invoke<AssistantEmptyTrashReport>("assistant_empty_trash", { request }),
  quoteMedia: (request: {
    sessionId: string;
    expectedOwner: string;
    expectedGeneration: number;
    params: AssistantVideoRequest;
  }) => invoke<AssistantMediaQuote>("assistant_quote_media", { request }),
  cancelVideo: (request: AssistantVideoActionRequest) =>
    invoke<void>("assistant_cancel_video", { request }),
  resumeVideo: (request: AssistantVideoActionRequest) =>
    invoke<void>("assistant_resume_video", { request }),
  saveMedia: (request: {
    sessionId: string;
    attachmentId: string;
    expectedOwner: string;
    expectedGeneration: number;
  }) => invoke<boolean>("assistant_save_media", { request }),
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
  /** Imports one short audio file into a temporary, private native slot. */
  pickAudio: (request: AssistantPasteTarget) =>
    invoke<AssistantAudioInput | null>("assistant_pick_audio", { request }),
  discardAudio: (request: AssistantPasteTarget & { inputId: string }) =>
    invoke<boolean>("assistant_discard_audio", { request }),
  transcribe: (request: AssistantTranscribeRequest) =>
    invoke<AssistantAudioResult>("assistant_transcribe_audio", {
      request: {
        sessionId: request.sessionId,
        expectedOwner: request.expectedOwner,
        expectedGeneration: request.expectedGeneration,
        runId: request.runId,
        operationId: request.operationId,
        inputId: request.inputId,
        modelId: request.modelId,
        ...(request.language ? { language: request.language } : {}),
        ...(request.keepOriginal ? { keepOriginal: true } : {}),
      },
    }),
  synthesize: (request: AssistantSpeechRequest) =>
    invoke<AssistantAudioResult>("assistant_synthesize_audio", {
      request: {
        sessionId: request.sessionId,
        expectedOwner: request.expectedOwner,
        expectedGeneration: request.expectedGeneration,
        runId: request.runId,
        operationId: request.operationId,
        modelId: request.modelId,
        input: request.input,
        voice: request.voice,
        responseFormat: request.responseFormat,
        ...(request.language ? { language: request.language } : {}),
        ...(request.instructions ? { instructions: request.instructions } : {}),
        ...(request.sourceMessageId
          ? { sourceMessageId: request.sourceMessageId }
          : {}),
      },
    }),
  /** Recording bytes use the same authority headers as a pasted file. */
  importAudio: async (file: File, target: AssistantPasteTarget) => {
    if (file.size > MAX_PASTED_FILE_BYTES)
      throw "assistant_attachment_too_large";
    const bytes = new Uint8Array(await file.arrayBuffer());
    return invoke<AssistantAudioInput>("assistant_import_audio", bytes, {
      headers: {
        "x-session-id": target.sessionId,
        "x-expected-owner": target.expectedOwner,
        "x-expected-generation": String(target.expectedGeneration),
        "x-file-name": headerFileName(file.name),
      },
    });
  },
  /**
   * Imports the files last dropped on the window into the conversation, with
   * a picked file's checks, all or nothing. Refused with
   * `assistant_drop_unavailable` for a token that is unknown, used, replaced
   * or older than 60 seconds; a token is spent once the import starts.
   */
  importDrop: (request: AssistantImportDropRequest) =>
    invoke<AssistantAttachment[]>("assistant_import_drop", { request }),
  /**
   * Imports one pasted file (`clipboardData.files[i]`) into the conversation.
   * One file per call; at most 10 per paste event, as in a pick. The bytes
   * go as a raw body and the rest as headers. Its type is read by the backend
   * from the bytes, never from `file.type`; a name that disagrees is fixed,
   * and a missing (or unencodable) name becomes `pasted-image-<time>.png`. A
   * file over 25 MB is refused with `assistant_attachment_too_large` before
   * it is read. The returned file is the attachment to add to the draft.
   */
  importPasted: async (file: File, target: AssistantPasteTarget) => {
    // Rejected with the code alone, as a command's failure is.
    if (file.size > MAX_PASTED_FILE_BYTES)
      throw "assistant_attachment_too_large";
    const bytes = new Uint8Array(await file.arrayBuffer());
    return invoke<AssistantAttachment[]>("assistant_import_pasted", bytes, {
      headers: {
        "x-session-id": target.sessionId,
        "x-expected-owner": target.expectedOwner,
        "x-expected-generation": String(target.expectedGeneration),
        "x-file-name": headerFileName(file.name),
      },
    });
  },
  /** A file drag entered or left the window. */
  onDrag: (
    callback: (state: AssistantDragState) => void,
  ): Promise<UnlistenFn> =>
    listen<{ state: AssistantDragState }>(ASSISTANT_DRAG_EVENT, ({ payload }) =>
      callback(payload.state),
    ),
  /** Files were dropped on the window; import them with `importDrop`. */
  onDrop: (callback: (drop: AssistantDrop) => void): Promise<UnlistenFn> =>
    listen<AssistantDrop>(ASSISTANT_DROP_EVENT, ({ payload }) =>
      callback(payload),
    ),
  send: (request: AssistantSendRequest) =>
    invoke<AssistantRunRef>("assistant_send", { request }),
  steer: (request: AssistantSteerRequest) =>
    invoke<null>("assistant_steer", { request }),
  stop: (sessionId: string, runId: string, operationId: string) =>
    invoke<null>("assistant_stop", { sessionId, runId, operationId }),
  /** Explicitly summarize native history as a separate billed run. */
  compact: (
    request: Omit<AssistantSwitchVersionRequest, "messageId"> & AssistantRunRef,
  ) => invoke<AssistantRunRef>("assistant_compact", { request }),
  forkSession: (request: AssistantSwitchVersionRequest & { title: string }) =>
    invoke<AssistantSessionDetail>("assistant_fork", { request }),
  /**
   * Shows another version of a turn, with the latest of what followed it,
   * and returns the conversation as it shows now: messages' `superseded`
   * says which are shown. Refused with `assistant_session_busy` while it
   * replies; the version shown already changes nothing.
   */
  switchVersion: (request: AssistantSwitchVersionRequest) =>
    invoke<AssistantSessionDetail>("assistant_switch_version", { request }),
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
  assistant_compact_unavailable: "compactUnavailable",
  assistant_fork_failed: "forkFailed",
  assistant_fork_unavailable: "forkUnavailable",
  assistant_fork_pending_media: "forkPendingMedia",
  assistant_version_unavailable: "versionUnavailable",
  assistant_version_switch_failed: "versionSwitchFailed",
  assistant_invalid_message: "invalidMessage",
  assistant_message_too_large: "messageTooLarge",
  assistant_title_too_long: "titleTooLong",
  assistant_instructions_too_long: "instructionsTooLong",
  assistant_invalid_thinking_level: "thinkingLevelUnavailable",
  assistant_attachment_not_found: "attachmentUnavailable",
  assistant_audio_unsupported: "audioUnsupported",
  assistant_audio_invalid: "audioInvalid",
  assistant_audio_too_long: "audioTooLong",
  assistant_audio_input_unavailable: "audioInputUnavailable",
  assistant_audio_invalid_params: "audioInvalidParams",
  assistant_audio_outcome_unknown: "audioOutcomeUnknown",
  assistant_microphone_denied: "microphoneDenied",
  assistant_microphone_unavailable: "microphoneUnavailable",
  assistant_recording_interrupted: "recordingInterrupted",
  assistant_audio_empty: "audioEmpty",
  assistant_attachment_too_many: "attachmentTooMany",
  assistant_attachment_too_large: "attachmentTooLarge",
  assistant_attachment_total_too_large: "attachmentTotalTooLarge",
  assistant_attachment_unsupported: "attachmentUnsupported",
  assistant_attachment_read_failed: "mediaReadFailed",
  assistant_attachment_unreadable: "attachmentUnreadable",
  assistant_attachment_write_failed: "attachmentWriteFailed",
  assistant_attachment_cleanup_failed: "attachmentCleanupFailed",
  assistant_attachment_dialog_failed: "attachmentDialogFailed",
  assistant_drop_unavailable: "dropUnavailable",
  assistant_attachment_not_in_backup: "attachmentNotInBackup",
  assistant_media_quote_expired: "mediaQuoteExpired",
  assistant_media_download_failed: "mediaDownloadFailed",
  assistant_media_job_not_found: "mediaJobNotFound",
  assistant_media_cancel_not_supported: "mediaCancelNotSupported",
  assistant_media_cancel_failed: "mediaCancelFailed",
  assistant_media_query_expired: "mediaQueryExpired",
  assistant_media_delete_pending: "mediaDeletePending",
  assistant_media_video_limit: "mediaVideoLimit",
  assistant_media_invalid_params: "mediaInvalidParams",
  assistant_media_outcome_unknown: "mediaOutcomeUnknown",
  assistant_media_safety_rejected: "mediaSafetyRejected",
  assistant_media_insufficient_credits: "mediaInsufficientCredits",
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

/** Account-scoped custom URI; never a filesystem path or an authenticated URL. */
export function assistantMediaUrl(
  sessionId: string,
  attachmentId: string,
  owner: string,
  generation: number,
): string {
  const base = isWindows()
    ? "http://assistant-media.localhost"
    : "assistant-media://localhost";
  return `${base}/${encodeURIComponent(sessionId)}/${encodeURIComponent(attachmentId)}?owner=${encodeURIComponent(owner)}&generation=${generation}`;
}
