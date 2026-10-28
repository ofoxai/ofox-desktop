import type {
  AssistantModel,
  AssistantThinkingLevel,
} from "@/lib/api/assistant";

/** Listed first, in this order; every other vendor follows alphabetically. */
const VENDOR_ORDER = ["openai", "anthropic", "google", "deepseek", "qwen"];

/** The vendor a model is listed under: the backend's, else its id's prefix. */
export function modelVendor(model: Pick<AssistantModel, "id" | "vendor">) {
  if (model.vendor !== undefined) return model.vendor;
  const slash = model.id.indexOf("/");
  return slash > 0 ? model.id.slice(0, slash) : "";
}

export interface VendorGroup {
  /** The raw prefix; empty for models without one. */
  vendor: string;
  /** What the picker's heading says. */
  label: string;
  /** Newest first. */
  models: AssistantModel[];
}

const rank = (vendor: string) => {
  const fixed = VENDOR_ORDER.indexOf(vendor);
  return fixed >= 0 ? fixed : VENDOR_ORDER.length;
};

/**
 * The models as the picker lists them: by vendor (the fixed few first, then
 * the rest by name, models with no vendor last), each vendor's models newest
 * first. Models the catalog gives no date, or the same one, keep their order.
 */
export function groupModelsByVendor(
  models: ReadonlyArray<AssistantModel>,
  vendorLabel: (vendor: string) => string,
  locale?: string,
): VendorGroup[] {
  const byVendor = new Map<string, AssistantModel[]>();
  for (const model of models) {
    const vendor = modelVendor(model);
    byVendor.set(vendor, [...(byVendor.get(vendor) ?? []), model]);
  }
  return [...byVendor.entries()]
    .map(([vendor, members]) => ({
      vendor,
      label: vendorLabel(vendor),
      models: [...members].sort(
        (a, b) =>
          (b.created ?? Number.NEGATIVE_INFINITY) -
            (a.created ?? Number.NEGATIVE_INFINITY) || 0,
      ),
    }))
    .sort((a, b) => {
      if (!a.vendor || !b.vendor) return Number(!a.vendor) - Number(!b.vendor);
      return (
        rank(a.vendor) - rank(b.vendor) ||
        a.label.localeCompare(b.label, locale)
      );
    });
}

/** A thinking menu entry: the default (no level sent) or one of the model's levels. */
export type ThinkingChoice = "default" | AssistantThinkingLevel;

/** The entries of the thinking menu; none when the model offers no levels. */
export function thinkingChoices(
  model: Pick<AssistantModel, "thinkingLevels"> | undefined,
): ThinkingChoice[] {
  const levels = model?.thinkingLevels ?? [];
  return levels.length > 0 ? ["default", ...levels] : [];
}

/**
 * The entry a conversation is on. A saved level its model no longer offers is
 * not sent, so it reads as the default.
 */
export function currentThinking(
  model: Pick<AssistantModel, "thinkingLevels"> | undefined,
  level: AssistantThinkingLevel | null | undefined,
): ThinkingChoice {
  return level && thinkingChoices(model).includes(level) ? level : "default";
}
