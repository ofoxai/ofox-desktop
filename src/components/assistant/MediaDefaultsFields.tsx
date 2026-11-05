import type { ReactNode } from "react";
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

/**
 * The model each kind starts with while the user has chosen none, when the
 * catalog offers it (the host's `PREFERRED_MEDIA_MODEL_IDS`).
 */
const PREFERRED_GENERATION_MODELS: Partial<
  Record<AssistantGenerationKind, string>
> = {
  image: "openai/gpt-image-2",
  video: "alibaba/wan-3.0",
};

/**
 * The model a generation uses: the chosen one while it exists, else the
 * preferred one, else the first.
 */
export function generationModel(
  models: ReadonlyArray<AssistantModel>,
  kind: AssistantGenerationKind,
  chosen?: string | null,
): AssistantModel | undefined {
  const usable = generationModels(models, kind);
  const find = (id?: string | null) => usable.find((model) => model.id === id);
  return find(chosen) ?? find(PREFERRED_GENERATION_MODELS[kind]) ?? usable[0];
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
  labeled,
}: {
  kind: AssistantGenerationKind;
  models: ReadonlyArray<AssistantModel>;
  value: MediaFieldsValue;
  onChange: (value: MediaFieldsValue) => void;
  disabled?: boolean;
  /** Each control under its name, two to a row: a form, not a toolbar. */
  labeled?: boolean;
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
  // A toolbar names a control on it ("Duration: 5 s"); a form above it.
  const field = (label: string, control: ReactNode) =>
    labeled ? (
      <div key={label} className="flex min-w-0 flex-col gap-1">
        <span className="text-note text-ink-muted">{label}</span>
        {control}
      </div>
    ) : (
      control
    );
  const shown = (label: string, current: string) =>
    labeled ? current : `${label}: ${current}`;
  const trigger = labeled ? "h-8 w-full max-w-none" : undefined;
  const pick = (
    label: string,
    current: string | null | undefined,
    values: string[],
    set: (next: string | undefined) => void,
    withDefault = true,
  ) =>
    values.length > 0 &&
    field(
      label,
      <MenuSelect
        aria-label={label}
        value={current ?? ""}
        valueLabel={shown(label, current || t("assistant.media.default"))}
        onChange={(next) => set(next || undefined)}
        disabled={disabled}
        triggerClassName={trigger}
        options={[
          ...(withDefault
            ? [{ value: "", label: t("assistant.media.default") }]
            : []),
          ...values.map((item) => ({ value: item, label: item })),
        ]}
      />,
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
          {caps.n &&
            field(
              t("assistant.media.count"),
              <MenuSelect
                aria-label={t("assistant.media.count")}
                value={String(image.n ?? 1)}
                valueLabel={shown(
                  t("assistant.media.count"),
                  String(image.n ?? 1),
                )}
                onChange={(next) => onChange({ ...image, n: Number(next) })}
                disabled={disabled}
                triggerClassName={trigger}
                options={[1, 2, 3, 4].map((n) => ({
                  value: String(n),
                  label: String(n),
                }))}
              />,
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
          {min !== null &&
            max !== null &&
            max >= min &&
            field(
              t("assistant.video.duration"),
              <MenuSelect
                aria-label={t("assistant.video.duration")}
                value={String(video.duration ?? "")}
                valueLabel={
                  video.duration
                    ? t("assistant.video.seconds", { count: video.duration })
                    : shown(
                        t("assistant.video.duration"),
                        t("assistant.media.default"),
                      )
                }
                disabled={disabled}
                triggerClassName={trigger}
                onChange={(next) =>
                  onChange({ ...video, duration: next ? Number(next) : null })
                }
                options={[
                  { value: "", label: t("assistant.media.default") },
                  ...Array.from(
                    { length: Math.min(max - min + 1, 120) },
                    (_, index) => ({
                      value: String(min + index),
                      label: t("assistant.video.seconds", {
                        count: min + index,
                      }),
                    }),
                  ),
                ]}
              />,
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
          {caps.supportsAudio &&
            field(
              t("assistant.generation.sound"),
              <label className="flex h-[26px] items-center gap-2 text-note text-ink-muted">
                <Switch
                  aria-label={t("assistant.generation.sound")}
                  checked={video.generateAudio ?? true}
                  onCheckedChange={(generateAudio) =>
                    onChange({ ...video, generateAudio })
                  }
                  disabled={disabled}
                />
                {labeled
                  ? t(
                      (video.generateAudio ?? true)
                        ? "assistant.generation.withSound"
                        : "assistant.generation.silent",
                    )
                  : t("assistant.generation.sound")}
              </label>,
            )}
        </>
      );
    }
    const caps = model.audio;
    const speech = value as AssistantSpeechDefaults;
    if (!caps) return null;
    return (
      <>
        {field(
          t("assistant.generation.voice"),
          <VoicePicker
            caps={caps}
            value={speech.voice}
            onChange={(voice) => onChange({ ...speech, voice })}
            label={t("assistant.generation.voice")}
            valueLabel={shown(
              t("assistant.generation.voice"),
              speech.voice || t("assistant.media.default"),
            )}
            disabled={disabled}
            triggerClassName={trigger}
          />,
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
    <div
      className={
        labeled ? "grid grid-cols-2 gap-3" : "flex flex-wrap items-center gap-2"
      }
    >
      {field(
        t("assistant.generation.model"),
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
          triggerClassName={trigger}
          options={options}
        />,
      )}
      {fields()}
    </div>
  );
}

/** The voice a model reads with: Gemini names one, ElevenLabs an id. */
const VOICE_FORMS: Record<string, RegExp> = {
  "gemini-tts": /^[A-Za-z]{1,64}$/,
  "elevenlabs-tts": /^[A-Za-z0-9]{20}$/,
};

/**
 * A model's voice: its known voices, or any other typed into the filter in
 * the form the model takes (a Gemini voice name, an ElevenLabs voice id).
 */
function VoicePicker({
  caps,
  value,
  onChange,
  label,
  valueLabel,
  disabled,
  triggerClassName,
}: {
  caps: NonNullable<AssistantModel["audio"]>;
  value: string | null | undefined;
  onChange: (voice: string | undefined) => void;
  label: string;
  valueLabel: string;
  disabled?: boolean;
  triggerClassName?: string;
}) {
  const { t } = useTranslation();
  const form = VOICE_FORMS[caps.family];
  const byId = caps.voiceKind === "id";
  // A voice chosen before, though no list has it, stays on the list.
  const voices = [
    ...new Set([...caps.suggestedVoices, ...(value ? [value] : [])]),
  ];
  return (
    <MenuSelect
      aria-label={label}
      value={value ?? ""}
      valueLabel={valueLabel}
      onChange={(next) => onChange(next || undefined)}
      disabled={disabled}
      triggerClassName={triggerClassName}
      searchable
      searchPlaceholder={t(
        byId
          ? "assistant.generation.voiceSearchId"
          : "assistant.generation.voiceSearchName",
      )}
      head={t(
        byId
          ? "assistant.generation.voiceIdHint"
          : "assistant.generation.voiceNameHint",
      )}
      createOption={(query) =>
        form?.test(query)
          ? {
              value: query,
              label: t("assistant.generation.useVoice", { voice: query }),
            }
          : null
      }
      options={[
        { value: "", label: t("assistant.media.default") },
        ...voices.map((voice) => ({ value: voice, label: voice })),
      ]}
    />
  );
}
