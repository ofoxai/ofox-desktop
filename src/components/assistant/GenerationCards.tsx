import { useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  Download,
  FileCode,
  FileJson,
  FileSpreadsheet,
  FileText,
  Info,
  Loader2,
  Presentation,
  RefreshCw,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";
import type {
  AssistantAttachment,
  AssistantDocumentIssue,
  AssistantGeneration,
  AssistantMessage,
  AssistantModel,
  AssistantRegenerateResult,
} from "@/lib/api/assistant";
import { assistantErrorKey, assistantMediaUrl } from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { Textarea } from "@/components/ui/textarea";
import { fmtBinaryBytes } from "@/lib/format";
import { MediaBlock, ActivityLink } from "./MediaBlock";
import { VideoBlock } from "./VideoBlock";
import { MEDIA_PLACEHOLDER, MediaToolButton, MediaTools } from "./MediaTools";
import { TopUpAction, needsTopUp } from "./TopUpAction";
import {
  MediaDefaultsFields,
  type MediaFieldsValue,
} from "./MediaDefaultsFields";

/** What can be made again with other parameters: all but a document. */
type MediaGeneration = Exclude<AssistantGeneration, { kind: "document" }>;

export interface RegenerateInput {
  prompt: string;
  model: string;
  params: Record<string, unknown>;
  confirmedCostUsd?: string | null;
  confirmUnknownCost?: boolean;
}

/** The fields a generation was made with, as its regeneration starts them. */
export function generationFields(
  generation: MediaGeneration,
): MediaFieldsValue {
  if (generation.kind === "image")
    return {
      model: generation.model,
      size: generation.params.size ?? undefined,
      quality: generation.params.quality ?? undefined,
      n: generation.params.n ?? undefined,
    };
  if (generation.kind === "video")
    return {
      model: generation.model,
      duration: generation.params.duration,
      resolution: generation.params.resolution,
      aspectRatio: generation.params.aspectRatio ?? undefined,
      generateAudio: generation.params.generateAudio,
    };
  return {
    model: generation.model,
    voice: generation.params.voice,
    format: generation.params.responseFormat,
  };
}

/** The request parameters for `fields` of a generation of `kind`. */
export function regenerationParams(
  generation: MediaGeneration,
  fields: MediaFieldsValue,
): Record<string, unknown> {
  const value = fields as Record<string, unknown>;
  const keys =
    generation.kind === "image"
      ? ["size", "quality", "n"]
      : generation.kind === "video"
        ? ["duration", "resolution", "aspectRatio", "generateAudio"]
        : ["voice", "format"];
  const params: Record<string, unknown> = {};
  for (const key of keys)
    if (value[key] !== undefined && value[key] !== null && value[key] !== "")
      params[key] = value[key];
  if (generation.kind === "speech") {
    if (generation.params.language)
      params.language = generation.params.language;
    if (generation.params.instructions)
      params.style = generation.params.instructions;
  }
  return params;
}

/** One line of what a generation used: model and parameters. */
function summary(
  generation: MediaGeneration,
  models: ReadonlyArray<AssistantModel>,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  const model = models.find((item) => item.id === generation.model);
  const name = model?.shortName || model?.name || generation.model;
  const params =
    generation.kind === "image"
      ? [
          generation.params.size,
          generation.params.quality,
          generation.params.n && generation.params.n > 1
            ? `×${generation.params.n}`
            : "",
        ]
      : generation.kind === "video"
        ? [
            t("assistant.video.seconds", { count: generation.params.duration }),
            generation.params.resolution,
            generation.params.aspectRatio,
            generation.params.generateAudio
              ? t("assistant.generation.withSound")
              : t("assistant.generation.silent"),
            generation.params.firstFrameId
              ? t(
                  generation.params.lastFrameId
                    ? "assistant.generation.firstLastFrames"
                    : "assistant.generation.firstFrame",
                )
              : generation.params.references?.length
                ? t("assistant.generation.references", {
                    count: generation.params.references.length,
                  })
                : "",
          ]
        : [generation.params.voice, generation.params.responseFormat];
  return [t(`assistant.generation.kinds.${generation.kind}`), name, ...params]
    .filter(Boolean)
    .join(" · ");
}

/**
 * What a generation used, sent and cost, behind a result's ⓘ: the model and
 * parameters, the cost, and the prompt (or the text read aloud).
 */
function GenerationDetails({
  item,
  models,
}: {
  item: MediaGeneration;
  models: ReadonlyArray<AssistantModel>;
}) {
  const { t } = useTranslation();
  const label = t("assistant.generation.details");
  return (
    <Popover>
      <PopoverTrigger asChild>
        <MediaToolButton label={label}>
          <Info aria-hidden />
        </MediaToolButton>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-label={label}
        className="w-80 max-w-[calc(100vw-32px)] space-y-2 p-3 text-note text-ink-2"
      >
        <p className="font-medium text-ink">{summary(item, models, t)}</p>
        {item.cost != null && (
          <p className="tabular-nums">
            {t("assistant.media.cost", { cost: item.cost })}
          </p>
        )}
        {item.prompt && (
          <div className="space-y-0.5 border-t border-line pt-2">
            <p className="text-ink-muted">
              {t(
                item.kind === "speech"
                  ? "assistant.generation.text"
                  : "assistant.generation.prompt",
              )}
            </p>
            <p className="max-h-48 overflow-y-auto whitespace-pre-wrap break-words">
              {item.prompt}
            </p>
          </div>
        )}
      </PopoverContent>
    </Popover>
  );
}

/**
 * A generation that failed, where its result would be: why, and what can be
 * done (top up, regenerate), in view rather than behind the pointer.
 */
function FailedResult({
  item,
  details,
  onRegenerate,
  disabled,
}: {
  item: AssistantGeneration;
  details: ReactNode;
  onRegenerate?: () => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <div className="w-full max-w-[420px] space-y-2 rounded-list border border-line bg-card-2 px-3 py-2.5">
      <p role="alert" className="text-note text-danger">
        {item.errorCode
          ? t(assistantErrorKey(item.errorCode))
          : item.kind === "video"
            ? t(`assistant.video.status.${item.status}`)
            : t("assistant.errors.generic")}
      </p>
      <div className="flex flex-wrap items-center gap-1">
        {needsTopUp(item.errorCode) && <TopUpAction />}
        {onRegenerate && (
          <Button variant="ghost" disabled={disabled} onClick={onRegenerate}>
            <RefreshCw aria-hidden className="h-3.5 w-3.5" />
            {t("assistant.generation.regenerate")}
          </Button>
        )}
        {details}
      </div>
    </div>
  );
}

/**
 * What the reply's generation tools made: each result alone, as it is. Its
 * actions (save, regenerate with other parameters, which goes straight to
 * the generation without the chat model, and its details) show while the
 * pointer or focus is on it; a failure says why where the result would be.
 */
export function GenerationCards({
  message,
  sessionId,
  owner,
  generation: authority,
  models,
  disabled,
  onSave,
  onSaveAndOpen,
  onCancelVideo,
  onResumeVideo,
  onRegenerate,
  sourceFile,
}: {
  message: AssistantMessage;
  sessionId: string;
  owner: string;
  generation: number;
  models: ReadonlyArray<AssistantModel>;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  /** Save a copy and open it with the default app. */
  onSaveAndOpen: (file: AssistantAttachment) => Promise<unknown>;
  onCancelVideo: (item: AssistantGeneration) => Promise<unknown>;
  onResumeVideo: (item: AssistantGeneration) => Promise<unknown>;
  onRegenerate?: (
    item: AssistantGeneration,
    input: RegenerateInput,
  ) => Promise<AssistantRegenerateResult | null>;
  /** A file of the conversation, such as an image a generation started from. */
  sourceFile?: (id: string) => AssistantAttachment | undefined;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<MediaGeneration | null>(null);
  const items = message.generations ?? [];
  if (items.length === 0) return null;
  return (
    <div className="mt-3 space-y-3">
      {items.map((item) => {
        const files = (message.attachments ?? []).filter((file) =>
          item.outputs.includes(file.id),
        );
        // A document has nothing to regenerate with other parameters, and no
        // details to show: its card is the file.
        if (item.kind === "document")
          return (
            <section
              key={item.toolCallId}
              aria-label={t("assistant.generation.kinds.document")}
              className="max-w-full"
            >
              {["error", "failed"].includes(item.status) ? (
                <FailedResult item={item} details={null} />
              ) : (
                <DocumentResult
                  item={item}
                  files={files}
                  disabled={disabled}
                  onSave={onSave}
                  onSaveAndOpen={onSaveAndOpen}
                />
              )}
            </section>
          );
        const regenerate = onRegenerate ? () => setEditing(item) : undefined;
        const details = <GenerationDetails item={item} models={models} />;
        const tools = (
          <>
            {regenerate && (
              <MediaToolButton
                label={t("assistant.generation.regenerate")}
                disabled={disabled || item.status === "submitting"}
                onClick={regenerate}
              >
                <RefreshCw aria-hidden />
              </MediaToolButton>
            )}
            {details}
          </>
        );
        return (
          <section
            key={item.toolCallId}
            aria-label={t(`assistant.generation.kinds.${item.kind}`)}
            className="max-w-full"
          >
            {["error", "failed"].includes(item.status) ? (
              <FailedResult
                item={item}
                details={details}
                onRegenerate={regenerate}
                disabled={disabled}
              />
            ) : item.kind === "image" ? (
              <MediaBlock
                message={message}
                media={item}
                sessionId={sessionId}
                owner={owner}
                generation={authority}
                disabled={disabled}
                onSave={onSave}
                tools={tools}
              />
            ) : item.kind === "video" ? (
              <VideoBlock
                poster={(() => {
                  const frame = generationSources(item, sourceFile, t).find(
                    ({ file }) =>
                      item.kind === "video" &&
                      file.id === item.params.firstFrameId,
                  );
                  return frame
                    ? assistantMediaUrl(
                        sessionId,
                        frame.file.id,
                        owner,
                        authority,
                      )
                    : undefined;
                })()}
                message={message}
                media={item}
                sessionId={sessionId}
                owner={owner}
                generation={authority}
                disabled={disabled}
                onSave={onSave}
                onCancel={() => onCancelVideo(item)}
                onResume={() => onResumeVideo(item)}
                tools={tools}
              />
            ) : (
              <SpeechResult
                item={item}
                files={files}
                url={(file) =>
                  assistantMediaUrl(sessionId, file.id, owner, authority)
                }
                disabled={disabled}
                onSave={onSave}
                tools={tools}
              />
            )}
          </section>
        );
      })}
      {editing && onRegenerate && (
        <RegenerateDialog
          item={editing}
          models={models}
          sources={generationSources(editing, sourceFile, t)}
          url={(file) =>
            assistantMediaUrl(sessionId, file.id, owner, authority)
          }
          onClose={() => setEditing(null)}
          onSubmit={(input) => onRegenerate(editing, input)}
        />
      )}
    </div>
  );
}

const DOCUMENT_ICONS: Record<string, LucideIcon> = {
  pptx: Presentation,
  xlsx: FileSpreadsheet,
  csv: FileSpreadsheet,
  html: FileCode,
  json: FileJson,
};
/** Most problems a card lists: the record keeps no more. */
const MAX_DOCUMENT_ISSUES = 20;

/** The problems a document's checks found, behind a count. */
function DocumentIssues({ issues }: { issues: AssistantDocumentIssue[] }) {
  const { t } = useTranslation();
  const label = t("assistant.document.issues", { count: issues.length });
  return (
    <Popover>
      <PopoverTrigger asChild>
        <Button
          variant="ghost"
          aria-label={label}
          title={label}
          className="h-6 flex-none gap-1 px-1.5 text-note text-warn"
        >
          <TriangleAlert aria-hidden className="h-3.5 w-3.5" />
          <span className="tabular-nums">{issues.length}</span>
        </Button>
      </PopoverTrigger>
      <PopoverContent
        align="end"
        aria-label={label}
        className="w-80 max-w-[calc(100vw-32px)] p-3 text-note text-ink-2"
      >
        <ul aria-label={label} className="max-h-60 space-y-2 overflow-y-auto">
          {issues.map((issue, index) => (
            <li key={index} className="space-y-0.5">
              <p className="break-words">
                {issue.severity && (
                  <span className="font-medium text-ink">
                    {t(
                      `assistant.document.severity.${issue.severity.toLowerCase()}`,
                      { defaultValue: issue.severity },
                    )}
                    {": "}
                  </span>
                )}
                {issue.message}
              </p>
              {issue.path && (
                <p className="break-all text-ink-muted">{issue.path}</p>
              )}
            </li>
          ))}
        </ul>
      </PopoverContent>
    </Popover>
  );
}

/**
 * A generated document as a card: what it is, how big, how many pages, slides
 * or sheets, and what its checks found; saved and opened in one step, or only
 * saved. There is nothing to play or preview here.
 */
function DocumentResult({
  item,
  files,
  disabled,
  onSave,
  onSaveAndOpen,
}: {
  item: Extract<AssistantGeneration, { kind: "document" }>;
  files: AssistantAttachment[];
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  onSaveAndOpen: (file: AssistantAttachment) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  if (item.status === "submitting")
    return (
      <div role="status" className={`${MEDIA_PLACEHOLDER} h-14 w-64 flex-row`}>
        <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
        {t("assistant.document.making")}
      </div>
    );
  const { pages, slides, sheets } = item.params;
  const issues = (item.params.issues ?? []).slice(0, MAX_DOCUMENT_ISSUES);
  const counts = (
    [
      ["slides", slides],
      ["pages", pages],
      ["sheets", sheets],
    ] as const
  ).flatMap(([key, count]) =>
    typeof count === "number"
      ? [t(`assistant.document.${key}`, { count })]
      : [],
  );
  const save = (
    action: (file: AssistantAttachment) => Promise<unknown>,
    file: AssistantAttachment,
  ) => {
    setSaving(true);
    void action(file)
      .catch((error) => toast.error(t(assistantErrorKey(error))))
      .finally(() => setSaving(false));
  };
  return (
    <div className="space-y-2">
      {files.map((file) => {
        const format = (
          item.params.format ||
          file.name.split(".").pop() ||
          ""
        ).toLowerCase();
        const Icon = DOCUMENT_ICONS[format] ?? FileText;
        return (
          <div
            key={file.id}
            className="w-full max-w-[420px] space-y-2.5 rounded-list border border-line bg-card-2 px-3 py-2.5"
          >
            <div className="flex items-center gap-3">
              <span className="flex h-10 w-10 flex-none items-center justify-center rounded-ctl border border-line bg-card text-ink-2">
                <Icon aria-hidden className="h-5 w-5" />
              </span>
              <div className="min-w-0 flex-1">
                <p
                  className="truncate text-ui font-medium text-ink"
                  title={file.name}
                >
                  {file.name}
                </p>
                <p className="truncate text-note tabular-nums text-ink-muted">
                  {[
                    format.toUpperCase(),
                    fmtBinaryBytes(file.sizeBytes),
                    ...counts,
                  ]
                    .filter(Boolean)
                    .join(" · ")}
                </p>
              </div>
              {issues.length > 0 && <DocumentIssues issues={issues} />}
            </div>
            <div className="flex flex-wrap items-center gap-2">
              <Button
                disabled={disabled || saving}
                onClick={() => save(onSaveAndOpen, file)}
              >
                {t("assistant.document.saveAndOpen")}
              </Button>
              <Button
                variant="secondary"
                disabled={disabled || saving}
                onClick={() => save(onSave, file)}
              >
                {t("assistant.media.save")}
              </Button>
            </div>
          </div>
        );
      })}
    </div>
  );
}

function SpeechResult({
  item,
  files,
  url,
  disabled,
  onSave,
  tools,
}: {
  item: AssistantGeneration;
  files: AssistantAttachment[];
  url: (file: AssistantAttachment) => string;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  tools: ReactNode;
}) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  if (item.status === "submitting")
    return (
      <div
        role="status"
        className={`${MEDIA_PLACEHOLDER} h-9 w-64 flex-row rounded-full`}
      >
        <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
        {t("assistant.generation.speaking")}
      </div>
    );
  if (item.status === "unknown")
    return (
      <p
        role="alert"
        className="max-w-[420px] rounded-list border border-line bg-card-2 px-3 py-2.5 text-note text-ink-muted"
      >
        {t("assistant.errors.mediaOutcomeUnknown")} <ActivityLink />
      </p>
    );
  return (
    <div className="space-y-2">
      {files.map((file) => (
        // A player has no picture to put the actions on: they follow it.
        <div
          key={file.id}
          className="group/media flex max-w-full items-center gap-1.5"
        >
          <audio
            aria-label={file.name}
            controls
            preload="metadata"
            src={url(file)}
            className="h-9 min-w-0 max-w-full"
          />
          <MediaTools inline>
            <MediaToolButton
              label={t("assistant.media.save")}
              disabled={disabled || saving}
              onClick={() => {
                setSaving(true);
                void onSave(file)
                  .catch((error) => toast.error(t(assistantErrorKey(error))))
                  .finally(() => setSaving(false));
              }}
            >
              <Download aria-hidden />
            </MediaToolButton>
            {tools}
          </MediaTools>
        </div>
      ))}
    </div>
  );
}

/**
 * The images a generation started from, each named by its part (the image
 * edited, a video's first or last frame, a reference). Only images a
 * thumbnail can show; a file that is gone is left out.
 */
function generationSources(
  item: MediaGeneration,
  sourceFile: ((id: string) => AssistantAttachment | undefined) | undefined,
  t: (key: string) => string,
): { file: AssistantAttachment; label: string }[] {
  const parts: [string | null | undefined, string][] =
    item.kind === "image"
      ? item.inputIds.map((id) => [id, t("assistant.generation.sourceImage")])
      : item.kind === "video"
        ? [
            [item.params.firstFrameId, t("assistant.video.firstFrameId")],
            [item.params.lastFrameId, t("assistant.video.lastFrameId")],
            ...(item.params.references ?? [])
              .filter((reference) => reference.kind === "image")
              .map((reference): [string, string] => [
                reference.id,
                t("assistant.generation.reference"),
              ]),
          ]
        : [];
  return parts.flatMap(([id, label]) => {
    const file = id ? sourceFile?.(id) : undefined;
    return file &&
      file.status !== "missing" &&
      file.mimeType.startsWith("image/")
      ? [{ file, label }]
      : [];
  });
}

/**
 * Change a generation's prompt, model or parameters and make it again. A
 * video shows its estimate and is made only once the user confirms it.
 */
function RegenerateDialog({
  item,
  models,
  sources,
  url,
  onClose,
  onSubmit,
}: {
  item: MediaGeneration;
  models: ReadonlyArray<AssistantModel>;
  sources: { file: AssistantAttachment; label: string }[];
  url: (file: AssistantAttachment) => string;
  onClose: () => void;
  onSubmit: (
    input: RegenerateInput,
  ) => Promise<AssistantRegenerateResult | null>;
}) {
  const { t } = useTranslation();
  const [prompt, setPrompt] = useState(item.prompt ?? "");
  const [fields, setFields] = useState<MediaFieldsValue>(() =>
    generationFields(item),
  );
  const [busy, setBusy] = useState(false);
  const [estimate, setEstimate] = useState<
    { cost: string | null; key: string } | undefined
  >();
  const model = fields.model ?? item.model;
  const params = regenerationParams(item, fields);
  // A confirmed estimate holds only for what it was given for.
  const key = JSON.stringify([prompt, model, params]);
  const confirmed = estimate?.key === key ? estimate : undefined;
  const submit = async () => {
    setBusy(true);
    try {
      const result = await onSubmit({
        prompt: prompt.trim(),
        model,
        params,
        ...(confirmed
          ? {
              confirmedCostUsd: confirmed.cost,
              confirmUnknownCost: confirmed.cost === null,
            }
          : {}),
      });
      if (result && "needsConfirmation" in result) {
        setEstimate({ cost: result.estimatedCostUsd, key });
        return;
      }
      if (result) onClose();
    } finally {
      setBusy(false);
    }
  };
  const promptLabel = t(
    item.kind === "speech"
      ? "assistant.generation.text"
      : "assistant.generation.prompt",
  );
  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent className="max-w-[620px]">
        <DialogHeader>
          <DialogTitle>{t("assistant.generation.regenerateTitle")}</DialogTitle>
          <DialogDescription>
            {t("assistant.generation.regenerateNote")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-4 overflow-y-auto px-[18px] py-4">
          <div className="flex items-start gap-3">
            {sources.length > 0 && (
              <div className="flex flex-none flex-col gap-2 pt-5">
                {sources.map(({ file, label }) => (
                  <figure
                    key={`${label}-${file.id}`}
                    className="relative h-16 w-16 overflow-hidden rounded-ctl border border-line bg-card-2"
                  >
                    <img
                      src={url(file)}
                      alt={label}
                      title={file.name}
                      className="h-full w-full object-cover"
                    />
                    <figcaption
                      aria-hidden
                      className="absolute bottom-0.5 left-0.5 rounded-[4px] bg-black/55 px-1 text-[10px] leading-4 text-white"
                    >
                      {label}
                    </figcaption>
                  </figure>
                ))}
              </div>
            )}
            <div className="flex min-w-0 flex-1 flex-col gap-1">
              <span aria-hidden className="text-note text-ink-muted">
                {promptLabel}
              </span>
              <div className="relative">
                <PromptField
                  label={promptLabel}
                  value={prompt}
                  onChange={setPrompt}
                />
                <span className="pointer-events-none absolute bottom-1.5 right-3 text-micro tabular-nums text-ink-faint">
                  {t("assistant.generation.characters", {
                    count: [...prompt].length,
                  })}
                </span>
              </div>
            </div>
          </div>
          <MediaDefaultsFields
            kind={item.kind}
            models={models}
            value={fields}
            onChange={setFields}
            disabled={busy}
            labeled
          />
        </div>
        <DialogFooter className="sm:justify-between">
          {/* What it will cost, once the host has estimated it. */}
          <p role="status" className="min-w-0 text-note text-ink-2">
            {confirmed &&
              (confirmed.cost
                ? t("assistant.generation.estimate", { cost: confirmed.cost })
                : t("assistant.generation.estimateUnknown"))}
          </p>
          <div className="flex gap-2">
            <Button variant="ghost" disabled={busy} onClick={onClose}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={busy || !prompt.trim()}
              onClick={() => void submit()}
            >
              {confirmed
                ? t("assistant.generation.confirmGenerate")
                : t("assistant.generation.generateAgain")}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** A prompt as tall as its text, up to about ten lines, then scrolled. */
function PromptField({
  label,
  value,
  onChange,
}: {
  label: string;
  value: string;
  onChange: (value: string) => void;
}) {
  const field = useRef<HTMLTextAreaElement>(null);
  useLayoutEffect(() => {
    const input = field.current;
    if (!input) return;
    // Cleared first, so deleting text can shrink it.
    input.style.height = "auto";
    input.style.height = `${Math.max(96, Math.min(280, input.scrollHeight))}px`;
  }, [value]);
  return (
    <Textarea
      ref={field}
      aria-label={label}
      value={value}
      onChange={(event) => onChange(event.target.value)}
      className="resize-none pb-6 text-ui leading-relaxed"
    />
  );
}
