import { useMemo } from "react";
import { useTranslation } from "react-i18next";
import type { AssistantModel, AssistantModelKind } from "@/lib/api/assistant";
import { fmtTokens } from "@/lib/format";
import { groupModelsByVendor } from "@/utils/assistantModels";
import type { MenuSelectOption } from "@/components/ui/menu-select";

const kindOf = (model: AssistantModel): AssistantModelKind =>
  model.kind ?? "chat";

/**
 * The models of one kind as a grouped menu lists them: by vendor, one line
 * each (name, context and prices; the rest in its tooltip). Shared by the
 * header's picker and the retry card's "another model" menu.
 */
export function useModelOptions(
  models: ReadonlyArray<AssistantModel>,
  kind: AssistantModelKind = "chat",
): MenuSelectOption[] {
  const { t, i18n } = useTranslation();
  return useMemo<MenuSelectOption[]>(() => {
    const vendorLabel = (vendor: string) =>
      vendor
        ? t(`assistant.vendors.${vendor}`, { defaultValue: vendor })
        : t("assistant.vendorOther");
    return groupModelsByVendor(
      models.filter((model) => kindOf(model) === kind),
      vendorLabel,
      i18n.language,
    ).flatMap(({ vendor, label, models: members }) =>
      members.map((model) => {
        if (kind === "image") {
          const caps = model.image;
          const price = caps?.perImagePrice
            ? t("assistant.media.perImage", { price: caps.perImagePrice })
            : caps?.tokenPricing
              ? t("assistant.media.imageTokenPrice", {
                  price: caps.tokenPricing.outputImage,
                })
              : "";
          return {
            value: model.id,
            label: model.shortName || model.name,
            meta: price || undefined,
            title: [
              model.name,
              price,
              caps?.edits ? t("assistant.media.canEdit") : "",
            ]
              .filter(Boolean)
              .join(" · "),
            group: vendor,
            groupLabel: label,
          };
        }
        if (kind === "video") {
          const caps = model.video;
          const price =
            caps?.fromPricePerSecond != null
              ? t("assistant.video.fromPrice", {
                  price: caps.fromPricePerSecond,
                })
              : "";
          // The price beside the name, the rest on a line of its own: all of
          // it on one line leaves the name a few letters.
          const specs = [
            caps?.minDuration != null && caps.maxDuration != null
              ? t("assistant.video.durationRange", {
                  min: caps.minDuration,
                  max: caps.maxDuration,
                })
              : "",
            caps?.resolutions.join(" / ") ?? "",
            caps?.supportsAudio ? t("assistant.video.audioCapability") : "",
          ].filter(Boolean);
          return {
            value: model.id,
            label: model.shortName || model.name,
            meta: price || undefined,
            note: specs.join(" · ") || undefined,
            title: [model.name, price, ...specs].filter(Boolean).join(" · "),
            group: vendor,
            groupLabel: label,
          };
        }
        if (kind === "speech" && model.audio) {
          // A speech model is billed per character read, or per token of the
          // text in and the audio out; never by its chat prices (0 / 0).
          const pricing = model.audio.pricing as
            | NonNullable<AssistantModel["audio"]>["pricing"]
            | undefined;
          const perThousand = (rate: string) =>
            String(Number((Number(rate) * 1000).toPrecision(6)));
          const price =
            pricing?.unit === "characters" && pricing.inputTextPerCharacter
              ? t("assistant.media.perThousandChars", {
                  price: perThousand(pricing.inputTextPerCharacter),
                })
              : pricing?.unit === "tokens" &&
                  pricing.inputTextPerMillion &&
                  pricing.outputAudioPerMillion
                ? t("assistant.media.audioTokenPrice", {
                    input: pricing.inputTextPerMillion,
                    output: pricing.outputAudioPerMillion,
                  })
                : "";
          const long =
            pricing?.unit === "tokens" && price
              ? t("assistant.media.audioTokenPriceLong", {
                  input: pricing.inputTextPerMillion,
                  output: pricing.outputAudioPerMillion,
                })
              : price;
          return {
            value: model.id,
            label: model.shortName || model.name,
            meta: price || undefined,
            title: [model.name, long].filter(Boolean).join("\n"),
            group: vendor,
            groupLabel: label,
          };
        }
        const tokens = model.contextWindow
          ? fmtTokens(model.contextWindow)
          : "";
        const prices = model.inputPricePerMillion
          ? {
              input: model.inputPricePerMillion,
              output: model.outputPricePerMillion ?? "—",
            }
          : null;
        const detail = [
          tokens ? t("assistant.contextWindow", { tokens }) : "",
          prices ? t("assistant.modelPrice", prices) : "",
          model.reasoning ? t("assistant.modelTags.reasoning") : "",
          model.supportsImages ? t("assistant.modelTags.vision") : "",
        ].filter(Boolean);
        return {
          value: model.id,
          label: model.shortName || model.name,
          meta:
            [tokens, prices ? t("assistant.modelPriceShort", prices) : ""]
              .filter(Boolean)
              .join(" · ") || undefined,
          title: [model.name, detail.join(" · ")].filter(Boolean).join("\n"),
          group: vendor,
          groupLabel: label,
        };
      }),
    );
    // `t` changes with the language, which is all `i18n.language` adds.
  }, [models, kind, t, i18n.language]);
}
