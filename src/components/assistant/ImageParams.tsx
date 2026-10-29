import { useTranslation } from "react-i18next";
import type {
  AssistantModel,
  AssistantImageRequest,
} from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";

export function ImageParams({
  model,
  value,
  onChange,
  disabled,
}: {
  model: AssistantModel;
  value: AssistantImageRequest;
  onChange: (value: AssistantImageRequest) => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const caps = model.image;
  if (!caps) return null;
  const select = (key: "size" | "quality", values: string[]) =>
    values.length > 0 && (
      <MenuSelect
        key={key}
        aria-label={t(`assistant.media.${key}`)}
        value={value[key] ?? ""}
        valueLabel={`${t(`assistant.media.${key}`)}: ${value[key] || t("assistant.media.default")}`}
        onChange={(next) => onChange({ ...value, [key]: next || undefined })}
        disabled={disabled}
        placeholder={t(`assistant.media.${key}`)}
        options={[
          { value: "", label: t("assistant.media.default") },
          ...values.map((v) => ({ value: v, label: v })),
        ]}
      />
    );
  const cost =
    caps.perImagePrice === null
      ? null
      : Number(caps.perImagePrice) * (value.n ?? 1);
  return (
    <div className="flex flex-wrap items-center gap-2 px-3 pb-2">
      {select("size", caps.sizes)}
      {select("quality", caps.qualities)}
      {caps.n && (
        <MenuSelect
          aria-label={t("assistant.media.count")}
          value={String(value.n ?? 1)}
          valueLabel={`${t("assistant.media.count")}: ${value.n ?? 1}`}
          onChange={(v) => onChange({ ...value, n: Number(v) })}
          disabled={disabled}
          options={Array.from({ length: 10 }, (_, i) => ({
            value: String(i + 1),
            label: String(i + 1),
          }))}
        />
      )}
      <span className="text-note text-ink-muted">
        {cost !== null && Number.isFinite(cost)
          ? t("assistant.media.estimate", {
              cost:
                cost
                  .toFixed(6)
                  .replace(/\.?0+$/, " ")
                  .trim() || "0",
            })
          : t("assistant.media.tokenCost")}
      </span>
    </div>
  );
}
