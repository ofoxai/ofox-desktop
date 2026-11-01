import { useTranslation } from "react-i18next";
import type {
  AssistantModel,
  AssistantThinkingLevel,
} from "@/lib/api/assistant";
import {
  currentThinking,
  thinkingChoices,
  type ThinkingChoice,
} from "@/utils/assistantModels";
import { MenuSelect } from "@/components/ui/menu-select";

/**
 * The composer's thinking control: the default or one of the levels the
 * model offers, with what deeper thinking costs said in the menu's head.
 * Nothing at all for a model that offers no levels.
 */
export function ThinkingMenu({
  model,
  level,
  disabled,
  onChange,
}: {
  model: AssistantModel | undefined;
  level: AssistantThinkingLevel | null | undefined;
  disabled: boolean;
  onChange: (choice: ThinkingChoice) => void;
}) {
  const { t } = useTranslation();
  const choices = thinkingChoices(model);
  if (choices.length === 0) return null;
  const current = currentThinking(model, level);
  const name = (choice: ThinkingChoice) =>
    choice === "default" ? t("effort.default") : t(`effort.levels.${choice}`);
  return (
    <MenuSelect<ThinkingChoice>
      aria-label={t("assistant.thinking.label")}
      value={current}
      onChange={onChange}
      disabled={disabled}
      options={choices.map((choice) => ({
        value: choice,
        label: name(choice),
      }))}
      valueLabel={t("assistant.thinking.trigger", { level: name(current) })}
      head={t("assistant.thinking.costNote")}
      width={260}
      align="start"
      // As quiet as the Web search toggle beside it (a ghost Button).
      triggerClassName="h-[27px] border-transparent bg-transparent px-2 text-ink-ctl hover:bg-pill hover:text-ink data-[state=open]:bg-pill data-[state=open]:text-ink"
    />
  );
}
