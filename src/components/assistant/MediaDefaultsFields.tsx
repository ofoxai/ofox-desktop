import { useTranslation } from "react-i18next";
import type {
  AssistantGenerationKind,
  AssistantImageDefaults,
  AssistantModel,
  AssistantSpeechDefaults,
  AssistantVideoDefaults,
} from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";
import { Switch } from "@/components/ui/switch";
import { useModelOptions } from "./useModelOptions";

export type MediaFieldsValue =
  | AssistantImageDefaults
  | AssistantVideoDefaults
  | AssistantSpeechDefaults;

/** The usable models of a generation kind, in catalog order. */
export function generationModels(
  models: ReadonlyArray<AssistantModel>,
  kind: AssistantGenerationKind,
): AssistantModel[] {
  return models.filter((model) => {
    if (model.kind !== kind) return false;
    if (kind === "image") return Boolean(model.image?.generations);
    if (kind === "video") return Boolean(model.video);
    return Boolean(model.audio);
  });
}

/** The model a generation uses: the chosen one while it exists, else the first. */
export function generationModel(
  models: ReadonlyArray<AssistantModel>,
  kind: AssistantGenerationKind,
  chosen?: string | null,
): AssistantModel | undefined {
  const usable = generationModels(models, kind);
  return usable.find((model) => model.id === chosen) ?? usable[0];
}

/**
 * A generation's model and parameters: the composer chip's defaults, or what
 * a regeneration changes. "Default" leaves a parameter to the model.
 */
export function MediaDefaultsFields({
  kind,
  models,
  value,
  onChange,
  disabled,
}: {
  kind: AssistantGenerationKind;
  models: ReadonlyArray<AssistantModel>;
  value: MediaFieldsValue;
  onChange: (value: MediaFieldsValue) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const usable = generationModels(models, kind);
  const options = useModelOptions(usable, kind === "speech" ? "speech" : kind);
  const model = generationModel(models, kind, value.model);
  if (!model)
    return (
      <p className="text-note text-ink-muted">
        {t("assistant.generation.noModel")}
      </p>
    );
  const pick = (
    label: string,
    current: string | null | undefined,
    values: string[],
    set: (next: string | undefined) => void,
    withDefault = true,
  ) =>
    values.length > 0 && (
      <MenuSelect
        aria-label={label}
        value={current ?? ""}
        valueLabel={`${label}: ${current || t("assistant.media.default")}`}
        onChange={(next) => set(next || undefined)}
        disabled={disabled}
        options={[
          ...(withDefault
            ? [{ value: "", label: t("assistant.media.default") }]
            : []),
          ...values.map((item) => ({ value: item, label: item })),
        ]}
      />
    );
  const fields = () => {
    if (kind === "image") {
      const caps = model.image;
      const image = value as AssistantImageDefaults;
      if (!caps) return null;
      return (
        <>
          {pick(t("assistant.media.size"), image.size, caps.sizes, (size) =>
            onChange({ ...image, size }),
          )}
          {pick(
            t("assistant.media.quality"),
            image.quality,
            caps.qualities,
            (quality) => onChange({ ...image, quality }),
          )}
          {caps.n && (
            <MenuSelect
              aria-label={t("assistant.media.count")}
              value={String(image.n ?? 1)}
              valueLabel={`${t("assistant.media.count")}: ${image.n ?? 1}`}
              onChange={(next) => onChange({ ...image, n: Number(next) })}
              disabled={disabled}
              options={[1, 2, 3, 4].map((n) => ({
                value: String(n),
                label: String(n),
              }))}
            />
          )}
        </>
      );
    }
    if (kind === "video") {
      const caps = model.video;
      const video = value as AssistantVideoDefaults;
      if (!caps) return null;
      const { minDuration: min, maxDuration: max } = caps;
      return (
        <>
          {min !== null && max !== null && max >= min && (
            <MenuSelect
              aria-label={t("assistant.video.duration")}
              value={String(video.duration ?? "")}
              valueLabel={
                video.duration
                  ? t("assistant.video.seconds", { count: video.duration })
                  : `${t("assistant.video.duration")}: ${t("assistant.media.default")}`
              }
              disabled={disabled}
              onChange={(next) =>
                onChange({ ...video, duration: next ? Number(next) : null })
              }
              options={[
                { value: "", label: t("assistant.media.default") },
                ...Array.from(
                  { length: Math.min(max - min + 1, 120) },
                  (_, index) => ({
                    value: String(min + index),
                    label: t("assistant.video.seconds", { count: min + index }),
                  }),
                ),
              ]}
            />
          )}
          {pick(
            t("assistant.video.resolution"),
            video.resolution,
            caps.resolutions,
            (resolution) => onChange({ ...video, resolution }),
          )}
          {pick(
            t("assistant.video.aspectRatio"),
            video.aspectRatio,
            caps.aspectRatios,
            (aspectRatio) => onChange({ ...video, aspectRatio }),
          )}
          {caps.supportsAudio && (
            <label className="flex items-center gap-2 text-note text-ink-muted">
              <Switch
                aria-label={t("assistant.generation.sound")}
                checked={video.generateAudio ?? true}
                onCheckedChange={(generateAudio) =>
                  onChange({ ...video, generateAudio })
                }
                disabled={disabled}
              />
              {t("assistant.generation.sound")}
            </label>
          )}
        </>
      );
    }
    const caps = model.audio;
    const speech = value as AssistantSpeechDefaults;
    if (!caps) return null;
    return (
      <>
        {pick(
          t("assistant.generation.voice"),
          speech.voice,
          caps.suggestedVoices,
          (voice) => onChange({ ...speech, voice }),
        )}
        {pick(
          t("assistant.generation.format"),
          speech.format,
          caps.responseFormats.filter((format) => format !== "pcm"),
          (format) => onChange({ ...speech, format }),
        )}
      </>
    );
  };
  return (
    <div className="flex flex-wrap items-center gap-2">
      <MenuSelect
        aria-label={t("assistant.generation.model")}
        value={model.id}
        valueLabel={model.shortName || model.name}
        onChange={(next) =>
          // Parameters belong to a model: another model starts from its own.
          onChange({ model: next })
        }
        disabled={disabled}
        searchable={usable.length > 8}
        searchPlaceholder={t("assistant.searchModels")}
        width={360}
        options={options}
      />
      {fields()}
    </div>
  );
}
