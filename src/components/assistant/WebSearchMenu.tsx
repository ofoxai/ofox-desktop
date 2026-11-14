import { Globe } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantSearchProvider } from "@/lib/api/assistant";
import { MenuSelect } from "@/components/ui/menu-select";
import { cn } from "@/lib/utils";

export type WebSearchChoice = "off" | AssistantSearchProvider;

/** The most searches one reply makes (the runtime's MAX_SEARCHES). */
export const MAX_SEARCHES_PER_REPLY = 6;

const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumSignificantDigits: 3,
});

/**
 * What one Ofox search may cost: the gateway bills each search call, and one
 * search can make one or two (a search, then opening a page). A reply makes
 * at most MAX_SEARCHES_PER_REPLY searches.
 */
export function searchPriceNote(unitPriceUsd: string) {
  const price = Number(unitPriceUsd);
  return {
    low: usd.format(price),
    high: usd.format(price * 2),
    max: MAX_SEARCHES_PER_REPLY,
  };
}

/**
 * The composer's web search control: off, or on through Ofox (paid, while
 * the catalog prices it) or Exa (free, when the backend offers it). The
 * provider is the account's, the switch the conversation's; both apply from
 * the next message on. It can always be turned off, and on only while one
 * provider is usable, so a conversation never keeps search on when it can't
 * be seen to work. Lit only for a search that will happen.
 */
export function WebSearchMenu({
  on,
  provider,
  ofoxPrice,
  exaOffered,
  disabled,
  describedBy,
  onChange,
}: {
  on: boolean;
  provider: AssistantSearchProvider;
  /** USD per Ofox search; null while Ofox search is unavailable. */
  ofoxPrice: string | null;
  /** The backend offers Exa (it reports the account's provider). */
  exaOffered: boolean;
  disabled: boolean;
  describedBy?: string;
  onChange: (choice: WebSearchChoice) => void;
}) {
  const { t } = useTranslation();
  const value: WebSearchChoice = on ? provider : "off";
  const usable = (choice: AssistantSearchProvider) =>
    choice === "exa" ? exaOffered : ofoxPrice !== null;
  const lit = on && usable(provider);
  const locked = disabled || (!on && !usable("ofox") && !usable("exa"));
  const icon = <Globe aria-hidden />;
  const head =
    value === "exa"
      ? t("assistant.webSearchMenu.exaNote")
      : value === "ofox"
        ? ofoxPrice !== null
          ? t("assistant.webSearchPrice", searchPriceNote(ofoxPrice))
          : t("assistant.webSearchUnavailable")
        : t("assistant.webSearchMenu.head");
  return (
    <MenuSelect<WebSearchChoice>
      aria-label={t("assistant.webSearch")}
      aria-describedby={describedBy}
      value={value}
      onChange={onChange}
      disabled={locked}
      options={[
        { value: "off", label: t("assistant.webSearchMenu.off"), icon },
        {
          value: "ofox",
          label: t("assistant.webSearchMenu.ofox"),
          note:
            ofoxPrice !== null
              ? t(
                  "assistant.webSearchMenu.ofoxNote",
                  searchPriceNote(ofoxPrice),
                )
              : t("assistant.webSearchMenu.ofoxUnavailable"),
          // Can always be left, never chosen while unavailable.
          disabled: ofoxPrice === null && value !== "ofox",
          icon,
        },
        ...(exaOffered || value === "exa"
          ? [
              {
                value: "exa" as const,
                label: t("assistant.webSearchMenu.exa"),
                note: t("assistant.webSearchMenu.exaShort"),
                disabled: !exaOffered && value !== "exa",
                icon,
              },
            ]
          : []),
      ]}
      valueLabel={
        value === "off"
          ? t("assistant.webSearch")
          : t(`assistant.webSearchMenu.trigger.${value}`)
      }
      head={head}
      width={300}
      align="start"
      // As quiet as a ghost Button, and brand-tinted while it searches.
      triggerClassName={cn(
        "h-[27px] border-transparent bg-transparent px-2 text-ink-ctl hover:bg-pill hover:text-ink data-[state=open]:bg-pill data-[state=open]:text-ink",
        lit &&
          "bg-brand-soft text-brand-ink hover:bg-brand-soft hover:text-brand-ink data-[state=open]:bg-brand-soft data-[state=open]:text-brand-ink",
      )}
    />
  );
}
