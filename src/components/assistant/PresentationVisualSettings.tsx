import { Image } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  PRESENTATION_MAX_IMAGES,
  presentationImageChoice,
  presentationImagePolicy,
  type AssistantPresentationVisualPolicy,
  type PresentationImageChoice,
} from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";

const CHOICES: ReadonlyArray<PresentationImageChoice> = [
  "off",
  "auto",
  "exact",
];
const COUNTS = Array.from({ length: PRESENTATION_MAX_IMAGES }, (_, index) =>
  String(index + 1),
);

/**
 * A deck's images, beside its style: none, up to a ceiling the model fills
 * as it sees fit, or exactly the number asked for. The second menu holds that
 * number; the USD ceiling follows from it. Picking never calls an image model.
 */
export function PresentationVisualSettings({
  value,
  onChange,
  disabled = false,
}: {
  value: AssistantPresentationVisualPolicy;
  onChange: (value: AssistantPresentationVisualPolicy) => void;
  disabled?: boolean;
}) {
  const { t } = useTranslation();
  const choice = presentationImageChoice(value);
  return (
    <>
      <MenuSelect
        aria-label={t("assistant.visual.title")}
        value={choice}
        onChange={(next) => onChange(presentationImagePolicy(next, value))}
        options={CHOICES.map((id) => ({
          value: id,
          label: t(`assistant.visual.choices.${id}.label`),
          note: t(`assistant.visual.choices.${id}.note`),
          icon: <Image aria-hidden className="h-3.5 w-3.5" />,
        }))}
        valueLabel={t("assistant.visual.trigger", {
          choice: t(`assistant.visual.choices.${choice}.label`),
        })}
        head={t("assistant.visual.head")}
        width={300}
        disabled={disabled}
      />
      {choice !== "off" && (
        <MenuSelect
          aria-label={t(`assistant.visual.count.${choice}.title`)}
          value={String(value.maxImages)}
          onChange={(next) =>
            onChange(presentationImagePolicy(choice, value, Number(next)))
          }
          options={COUNTS.map((count) => ({
            value: count,
            label: t(`assistant.visual.count.${choice}.option`, {
              count: Number(count),
            }),
          }))}
          width={140}
          disabled={disabled}
        />
      )}
    </>
  );
}
