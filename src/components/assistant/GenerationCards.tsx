import { useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { RefreshCw } from "lucide-react";
import type {
  AssistantAttachment,
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
import { Textarea } from "@/components/ui/textarea";
import { MediaBlock, ActivityLink } from "./MediaBlock";
import { VideoBlock } from "./VideoBlock";
import { TopUpAction, needsTopUp } from "./TopUpAction";
import {
  MediaDefaultsFields,
  type MediaFieldsValue,
} from "./MediaDefaultsFields";

export interface RegenerateInput {
  prompt: string;
  model: string;
  params: Record<string, unknown>;
  confirmedCostUsd?: string | null;
  confirmUnknownCost?: boolean;
}

/** The fields a generation was made with, as its regeneration starts them. */
export function generationFields(
  generation: AssistantGeneration,
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
  generation: AssistantGeneration,
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
  generation: AssistantGeneration,
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
 * What the reply's generation tools made: each result with the model and
 * parameters it used, the prompt that was sent, and "regenerate with other
 * parameters", which goes straight to the generation without the chat model.
 */
export function GenerationCards({
  message,
  sessionId,
  owner,
  generation: authority,
  models,
  disabled,
  onSave,
  onCancelVideo,
  onResumeVideo,
  onRegenerate,
}: {
  message: AssistantMessage;
  sessionId: string;
  owner: string;
  generation: number;
  models: ReadonlyArray<AssistantModel>;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
  onCancelVideo: (item: AssistantGeneration) => Promise<unknown>;
  onResumeVideo: (item: AssistantGeneration) => Promise<unknown>;
  onRegenerate?: (
    item: AssistantGeneration,
    input: RegenerateInput,
  ) => Promise<AssistantRegenerateResult | null>;
}) {
  const { t } = useTranslation();
  const [editing, setEditing] = useState<AssistantGeneration | null>(null);
  const items = message.generations ?? [];
  if (items.length === 0) return null;
  return (
    <div className="mt-2 space-y-2">
      {items.map((item) => {
        const files = (message.attachments ?? []).filter((file) =>
          item.outputs.includes(file.id),
        );
        return (
          <section
            key={item.toolCallId}
            aria-label={t(`assistant.generation.kinds.${item.kind}`)}
            className="space-y-2 rounded-list border border-line bg-card-2 p-2"
          >
            <p className="text-note text-ink-muted">
              {summary(item, models, t)}
            </p>
            {item.kind === "image" && (
              <MediaBlock
                message={message}
                media={item}
                sessionId={sessionId}
                owner={owner}
                generation={authority}
                disabled={disabled}
                onSave={onSave}
              />
            )}
            {item.kind === "video" && (
              <VideoBlock
                message={message}
                media={item}
                sessionId={sessionId}
                owner={owner}
                generation={authority}
                disabled={disabled}
                onSave={onSave}
                onCancel={() => onCancelVideo(item)}
                onResume={() => onResumeVideo(item)}
              />
            )}
            {item.kind === "speech" && (
              <SpeechResult
                item={item}
                files={files}
                url={(file) =>
                  assistantMediaUrl(sessionId, file.id, owner, authority)
                }
                disabled={disabled}
                onSave={onSave}
              />
            )}
            {["error", "failed"].includes(item.status) && item.errorCode && (
              <p role="alert" className="text-note text-danger">
                {t(assistantErrorKey(item.errorCode))}
              </p>
            )}
            {["error", "failed"].includes(item.status) &&
              needsTopUp(item.errorCode) && <TopUpAction />}
            {item.prompt && (
              <details className="text-note text-ink-muted">
                <summary className="cursor-default select-none">
                  {t(
                    item.kind === "speech"
                      ? "assistant.generation.text"
                      : "assistant.generation.prompt",
                  )}
                </summary>
                <p className="mt-1 whitespace-pre-wrap break-words text-ink-2">
                  {item.prompt}
                </p>
              </details>
            )}
            {onRegenerate && (
              <Button
                variant="ghost"
                disabled={disabled || item.status === "submitting"}
                onClick={() => setEditing(item)}
              >
                <RefreshCw aria-hidden className="h-3.5 w-3.5" />
                {t("assistant.generation.regenerate")}
              </Button>
            )}
          </section>
        );
      })}
      {editing && onRegenerate && (
        <RegenerateDialog
          item={editing}
          models={models}
          onClose={() => setEditing(null)}
          onSubmit={(input) => onRegenerate(editing, input)}
        />
      )}
    </div>
  );
}

function SpeechResult({
  item,
  files,
  url,
  disabled,
  onSave,
}: {
  item: AssistantGeneration;
  files: AssistantAttachment[];
  url: (file: AssistantAttachment) => string;
  disabled?: boolean;
  onSave: (file: AssistantAttachment) => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const [saving, setSaving] = useState(false);
  if (item.status === "submitting")
    return (
      <p role="status" className="text-note text-ink-muted">
        {t("assistant.generation.speaking")}
      </p>
    );
  if (item.status === "unknown")
    return (
      <p role="alert" className="text-note text-ink-muted">
        {t("assistant.errors.mediaOutcomeUnknown")} <ActivityLink />
      </p>
    );
  return (
    <>
      {files.map((file) => (
        <div key={file.id} className="flex flex-wrap items-center gap-2">
          <audio
            aria-label={file.name}
            controls
            preload="metadata"
            src={url(file)}
            className="h-9 max-w-full"
          />
          <Button
            variant="ghost"
            disabled={disabled || saving}
            onClick={() => {
              setSaving(true);
              void onSave(file)
                .catch((error) => toast.error(t(assistantErrorKey(error))))
                .finally(() => setSaving(false));
            }}
          >
            {t("assistant.media.save")}
          </Button>
        </div>
      ))}
      {item.cost != null && (
        <p className="text-note tabular-nums text-ink-muted">
          {t("assistant.media.cost", { cost: item.cost })}
        </p>
      )}
    </>
  );
}

/**
 * Change a generation's prompt, model or parameters and make it again. A
 * video shows its estimate and is made only once the user confirms it.
 */
function RegenerateDialog({
  item,
  models,
  onClose,
  onSubmit,
}: {
  item: AssistantGeneration;
  models: ReadonlyArray<AssistantModel>;
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
  return (
    <Dialog open onOpenChange={(open) => !open && !busy && onClose()}>
      <DialogContent className="max-w-[560px]">
        <DialogHeader>
          <DialogTitle>{t("assistant.generation.regenerateTitle")}</DialogTitle>
          <DialogDescription>
            {t("assistant.generation.regenerateNote")}
          </DialogDescription>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-[18px] py-4">
          <Textarea
            rows={4}
            aria-label={t(
              item.kind === "speech"
                ? "assistant.generation.text"
                : "assistant.generation.prompt",
            )}
            value={prompt}
            onChange={(event) => setPrompt(event.target.value)}
            className="resize-none text-ui"
          />
          <MediaDefaultsFields
            kind={item.kind}
            models={models}
            value={fields}
            onChange={setFields}
            disabled={busy}
          />
          {confirmed && (
            <p role="status" className="text-note text-ink-2">
              {confirmed.cost
                ? t("assistant.generation.estimate", { cost: confirmed.cost })
                : t("assistant.generation.estimateUnknown")}
            </p>
          )}
        </div>
        <DialogFooter>
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
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
