import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantModel, AssistantModelKind } from "@/lib/api/assistant";
import { fmtTokens } from "@/lib/format";
import { groupModelsByVendor } from "@/utils/assistantModels";
import { MenuSelect, type MenuSelectOption } from "@/components/ui/menu-select";
import { SegmentedControl } from "@/components/ui/segmented";

/**
 * The kinds the picker's switch offers. Stage 9 adds "image" and "video"
 * here: their labels (`assistant.kinds.*`) are in place, and the options
 * below already follow the chosen kind.
 */
const KINDS: AssistantModelKind[] = ["chat"];

const kindOf = (model: AssistantModel): AssistantModelKind =>
  model.kind ?? "chat";

/**
 * The header's model picker: the models of one kind, grouped by vendor, with
 * what each costs and can do. `partial` says a catalog could not be read, so
 * the list may lack some models.
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
  const { t, i18n } = useTranslation();
  const [kind, setKind] = useState<AssistantModelKind>("chat");

  const options = useMemo<MenuSelectOption[]>(() => {
    const vendorLabel = (vendor: string) =>
      vendor
        ? t(`assistant.vendors.${vendor}`, { defaultValue: vendor })
        : t("assistant.vendorOther");
    return groupModelsByVendor(
      models.filter((model) => kindOf(model) === kind),
      vendorLabel,
      i18n.language,
    ).flatMap(({ vendor, label, models: members }) =>
      members.map((model) => ({
        value: model.id,
        label: model.shortName ?? model.name,
        note: [
          model.contextWindow
            ? t("assistant.contextWindow", {
                tokens: fmtTokens(model.contextWindow),
              })
            : "",
          model.inputPricePerMillion
            ? t("assistant.modelPrice", {
                input: model.inputPricePerMillion,
                output: model.outputPricePerMillion ?? "—",
              })
            : "",
        ]
          .filter(Boolean)
          .join(" · "),
        tags: [
          model.reasoning ? t("assistant.modelTags.reasoning") : "",
          model.supportsImages ? t("assistant.modelTags.vision") : "",
        ].filter(Boolean),
        group: vendor,
        groupLabel: label,
      })),
    );
    // `t` changes with the language, which is all `i18n.language` adds.
  }, [models, kind, t, i18n.language]);

  const count = (of: AssistantModelKind) =>
    models.filter((model) => kindOf(model) === of).length;
  const chosen = models.find((model) => model.id === value);

  return (
    <MenuSelect
      aria-label={t("assistant.model")}
      value={value}
      onChange={onChange}
      disabled={disabled}
      searchable
      searchPlaceholder={t("assistant.searchModels")}
      placeholder={placeholder}
      // The trigger names the model in full, vendor included.
      valueLabel={chosen?.name}
      width={310}
      wrapNotes
      align="end"
      triggerClassName="!max-w-[min(250px,100%)]"
      options={options}
      head={
        // A click here must not take the focus off the search field or the
        // highlighted option, where the keyboard works the list.
        <div
          className="grid gap-1.5"
          onMouseDown={(event) => event.preventDefault()}
        >
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
          {partial && <p>{t("assistant.modelsPartial")}</p>}
        </div>
      }
    />
  );
}
