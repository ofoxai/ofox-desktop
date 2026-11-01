import { useTranslation } from "react-i18next";
import type {
  AssistantModel,
  AssistantAttachment,
  AssistantVideoRequest,
} from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";
import { Switch } from "@/components/ui/switch";

export function defaultVideoParams(
  model: AssistantModel,
): AssistantVideoRequest {
  return {
    duration: model.video?.minDuration ?? 4,
    resolution:
      model.video?.defaultResolution ?? model.video?.resolutions[0] ?? "",
    generateAudio: false,
  };
}
export function videoFrames(files: AssistantAttachment[]) {
  return files.filter(
    (file) =>
      ["ready", "partial"].includes(file.status) &&
      ["image/png", "image/jpeg", "image/webp"].includes(file.mimeType) &&
      file.sizeBytes <= 15 * 1024 * 1024,
  );
}
export function VideoParams({
  model,
  value,
  attachments,
  onChange,
  disabled,
}: {
  model: AssistantModel;
  value: AssistantVideoRequest;
  attachments: AssistantAttachment[];
  onChange: (value: AssistantVideoRequest) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const caps = model.video;
  if (!caps) return null;
  const frames = videoFrames(attachments);
  const select = (key: "resolution" | "aspectRatio", values: string[]) =>
    values.length > 0 && (
      <MenuSelect
        key={key}
        aria-label={t(`assistant.video.${key}`)}
        value={value[key] ?? ""}
        valueLabel={`${t(`assistant.video.${key}`)}: ${value[key] || t("assistant.media.default")}`}
        onChange={(next) => onChange({ ...value, [key]: next || undefined })}
        disabled={disabled}
        options={[
          ...(key === "aspectRatio"
            ? [{ value: "", label: t("assistant.media.default") }]
            : []),
          ...values.map((item) => ({ value: item, label: item })),
        ]}
      />
    );
  const frameSelect = (key: "firstFrameId" | "lastFrameId") => (
    <MenuSelect
      aria-label={t(`assistant.video.${key}`)}
      value={value[key] ?? ""}
      valueLabel={`${t(`assistant.video.${key}`)}: ${frames.find((file) => file.id === value[key])?.name ?? t("assistant.video.none")}`}
      disabled={disabled || (key === "lastFrameId" && !value.firstFrameId)}
      options={[
        { value: "", label: t("assistant.video.none") },
        ...frames.map((file) => ({ value: file.id, label: file.name })),
      ]}
      onChange={(next) =>
        onChange({
          ...value,
          [key]: next || undefined,
          ...(key === "firstFrameId" && !next
            ? { lastFrameId: undefined }
            : {}),
        })
      }
    />
  );
  const min = caps.minDuration,
    max = caps.maxDuration;
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 pb-2">
      {min !== null && max !== null && max >= min && (
        <MenuSelect
          aria-label={t("assistant.video.duration")}
          value={String(value.duration)}
          valueLabel={t("assistant.video.seconds", { count: value.duration })}
          disabled={disabled}
          onChange={(next) => onChange({ ...value, duration: Number(next) })}
          options={Array.from(
            { length: Math.min(max - min + 1, 600) },
            (_, index) => ({
              value: String(min + index),
              label: t("assistant.video.seconds", { count: min + index }),
            }),
          )}
        />
      )}
      {select("resolution", caps.resolutions)}
      {select("aspectRatio", caps.aspectRatios)}
      {caps.supportsAudio && (
        <label className="flex items-center gap-2 text-note text-ink-muted">
          <Switch
            aria-label={t("assistant.video.generateAudio")}
            checked={value.generateAudio ?? false}
            onCheckedChange={(generateAudio) =>
              onChange({ ...value, generateAudio })
            }
            disabled={disabled}
          />
          {t("assistant.video.generateAudio")}
        </label>
      )}
      {caps.modes.includes("i2v") && (
        <>
          {frameSelect("firstFrameId")}
          {frameSelect("lastFrameId")}
        </>
      )}
    </div>
  );
}
