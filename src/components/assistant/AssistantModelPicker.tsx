import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantModel, AssistantModelKind } from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";
import { SegmentedControl } from "@/components/ui/segmented";
import { useModelOptions } from "./useModelOptions";

/**
 * The kinds the picker's switch offers. Stage 9 adds "image" and "video"
 * here: their labels (`assistant.kinds.*`) are in place, and the options
 * below already follow the chosen kind.
 */
const KINDS: AssistantModelKind[] = ["chat"];

const kindOf = (model: AssistantModel): AssistantModelKind =>
  model.kind ?? "chat";

/**
 * The header's model picker: the models of one kind, grouped by vendor, one
 * line each (name, context and prices; the rest in its tooltip). `partial`
 * says a catalog could not be read, so the list may lack some models.
 */
export function AssistantModelPicker({
  models,
  value,
  onChange,
  partial,
  disabled,
  placeholder,
}: {
  models: ReadonlyArray<AssistantModel>;
  value: string;
  onChange: (modelId: string) => void;
  partial: boolean;
  disabled: boolean;
  placeholder: string;
}) {
  const { t } = useTranslation();
  const [kind, setKind] = useState<AssistantModelKind>("chat");

  const options = useModelOptions(models, kind);

  const count = (of: AssistantModelKind) =>
    models.filter((model) => kindOf(model) === of).length;
  const chosen = models.find((model) => model.id === value);
  // A switch with one segment is no choice: it shows once there are more.
  const switchable = KINDS.length > 1;

  return (
    <MenuSelect
      aria-label={t("assistant.model")}
      value={value}
      onChange={onChange}
      disabled={disabled}
      searchable
      searchPlaceholder={t("assistant.searchModels")}
      placeholder={placeholder}
      // The trigger names the model as its row does (the vendor is its
      // group's); the tooltip names it in full.
      valueLabel={chosen ? chosen.shortName || chosen.name : undefined}
      triggerTitle={chosen?.name}
      width={410}
      menuClassName="max-h-[min(480px,70vh)]"
      align="end"
      triggerClassName="!max-w-[min(250px,100%)]"
      options={options}
      head={
        (switchable || partial) && (
          // A click here must not take the focus off the search field or the
          // highlighted option, where the keyboard works the list.
          <div
            className="grid gap-1.5"
            onMouseDown={(event) => event.preventDefault()}
          >
            {switchable && (
              <SegmentedControl
                aria-label={t("assistant.modelKind")}
                value={kind}
                onChange={setKind}
                options={KINDS.map((item) => ({
                  value: item,
                  label: (
                    <>
                      {t(`assistant.kinds.${item}`)}{" "}
                      <span className="tabular-nums text-ink-faint">
                        {count(item)}
                      </span>
                    </>
                  ),
                }))}
              />
            )}
            {partial && <p>{t("assistant.modelsPartial")}</p>}
          </div>
        )
      }
    />
  );
}
