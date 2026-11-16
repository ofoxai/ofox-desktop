import {
  useEffect,
  useId,
  useRef,
  useState,
  type KeyboardEvent,
  type RefObject,
} from "react";
import { Check, Eye, Search, Sparkles, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import type { AssistantPresentationStyleId } from "@/lib/api/assistant";
import {
  PRESENTATION_FAMILIES,
  PRESENTATION_STYLES,
  presentationFamilyName,
  presentationStyleIntro,
  presentationStyleName,
  presentationStyleScenarios,
  type PresentationStyle,
} from "@/lib/assistantPresentationStyles";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { SegmentedControl } from "@/components/ui/segmented";
import { cn } from "@/lib/utils";
import { PresentationStylePreview } from "./PresentationStylePreview";
import { presentationSource } from "@/lib/assistantPresentationSources";

/** A style's cover; a style with no native sample yet says so instead. */
function CoverImage({ src, alt }: { src: string | null; alt: string }) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(false);
  useEffect(() => setFailed(false), [src]);
  return !src || failed ? (
    <span className="flex aspect-video items-center justify-center bg-card-2 p-3 text-center text-note text-ink-muted">
      {t(
        src
          ? "assistant.presentation.previewUnavailable"
          : "assistant.presentation.nativeUnavailable",
      )}
    </span>
  ) : (
    <img
      src={src}
      alt={alt}
      width={640}
      height={360}
      loading="lazy"
      decoding="async"
      className="aspect-video w-full object-contain"
      onError={() => setFailed(true)}
    />
  );
}

const ALL_FAMILIES = "all";

/** Public local samples only. Browsing and choosing never starts generation. */
export function PresentationStylePicker({
  open,
  onOpenChange,
  value,
  onSelect,
  returnFocusRef,
  onCloseAutoFocus,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  value: AssistantPresentationStyleId;
  /** Called only by an explicit Use action, not by radio/preview navigation. */
  onSelect: (value: AssistantPresentationStyleId) => void;
  returnFocusRef?: RefObject<HTMLElement>;
  onCloseAutoFocus?: (event: Event) => void;
}) {
  const { t, i18n } = useTranslation();
  const language = i18n.language;
  const [choice, setChoice] = useState(value);
  const [query, setQuery] = useState("");
  const [family, setFamily] = useState(ALL_FAMILIES);
  const [preview, setPreview] = useState<PresentationStyle | null>(null);
  const search = useRef<HTMLInputElement>(null);
  const previewReturn = useRef<HTMLButtonElement | null>(null);
  const outerOpen = useRef(open);
  outerOpen.current = open;
  const radios = useRef(
    new Map<AssistantPresentationStyleId, HTMLButtonElement>(),
  );
  const groupLabel = useId();
  const autoDescription = useId();
  const selectionLabel = useId();
  useEffect(() => {
    if (open) {
      setChoice(value);
      setQuery("");
      setFamily(ALL_FAMILIES);
    }
    setPreview(null);
  }, [open, value]);
  const normalized = query.trim().toLocaleLowerCase();
  const styles = PRESENTATION_STYLES.filter(
    (style) =>
      (family === ALL_FAMILIES || style.family === family) &&
      [
        style.id,
        presentationStyleName(style.id, t, language),
        presentationStyleIntro(style, t, language),
        presentationStyleScenarios(style, t),
        presentationFamilyName(style.family, t),
      ]
        .join(" ")
        .toLocaleLowerCase()
        .includes(normalized),
  );
  const options: AssistantPresentationStyleId[] = [
    "auto",
    ...styles.map((style) => style.id),
  ];
  const focusedChoice = options.includes(choice) ? choice : "auto";
  const move = (
    event: KeyboardEvent<HTMLButtonElement>,
    id: AssistantPresentationStyleId,
  ) => {
    const at = options.indexOf(id);
    const next =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? options.length - 1
          : ["ArrowLeft", "ArrowUp"].includes(event.key)
            ? (at - 1 + options.length) % options.length
            : ["ArrowRight", "ArrowDown"].includes(event.key)
              ? (at + 1) % options.length
              : null;
    if (next === null) return;
    event.preventDefault();
    setChoice(options[next]);
    radios.current.get(options[next])?.focus();
  };
  const radioRef =
    (id: AssistantPresentationStyleId) =>
    (element: HTMLButtonElement | null) => {
      if (element) radios.current.set(id, element);
      else radios.current.delete(id);
    };
  const finish = (id: AssistantPresentationStyleId) => {
    setPreview(null);
    onSelect(id);
  };
  const closeFocus = (event: Event) => {
    if (onCloseAutoFocus) onCloseAutoFocus(event);
    else if (returnFocusRef?.current?.isConnected) {
      event.preventDefault();
      returnFocusRef.current.focus();
    }
  };
  return (
    <>
      <Dialog open={open} onOpenChange={onOpenChange}>
        <DialogContent
          className="w-[calc(100vw-32px)] max-w-[1100px] max-h-[88vh]"
          onCloseAutoFocus={closeFocus}
        >
          <DialogHeader className="pr-12">
            <DialogTitle>{t("assistant.presentation.title")}</DialogTitle>
            <DialogDescription>
              {t("assistant.presentation.description")}
            </DialogDescription>
            <Button
              variant="icon"
              aria-label={t("assistant.presentation.close")}
              className="absolute right-3 top-3"
              onClick={() => onOpenChange(false)}
            >
              <X aria-hidden className="h-4 w-4" />
            </Button>
          </DialogHeader>
          <div className="min-h-0 overflow-y-auto px-[18px] py-4">
            <div className="relative mb-3">
              <Search
                aria-hidden
                className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-ink-muted"
              />
              <Input
                ref={search}
                value={query}
                onChange={(event) => setQuery(event.target.value)}
                aria-label={t("assistant.presentation.search")}
                placeholder={t("assistant.presentation.searchPlaceholder")}
                className="pl-9 pr-9"
              />
              {query && (
                <Button
                  variant="icon"
                  aria-label={t("assistant.presentation.resetSearch")}
                  className="absolute right-1 top-1/2 h-7 w-7 -translate-y-1/2"
                  onClick={() => {
                    setQuery("");
                    search.current?.focus();
                  }}
                >
                  <X aria-hidden className="h-3.5 w-3.5" />
                </Button>
              )}
            </div>
            <SegmentedControl
              aria-label={t("assistant.presentation.familyFilter")}
              value={family}
              onChange={setFamily}
              className="mb-3 max-w-full flex-wrap"
              options={[
                {
                  value: ALL_FAMILIES,
                  label: t("assistant.presentation.allFamilies"),
                },
                ...PRESENTATION_FAMILIES.map((id) => ({
                  value: id,
                  label: presentationFamilyName(id, t),
                })),
              ]}
            />
            <p id={groupLabel} className="sr-only">
              {t("assistant.presentation.options")}
            </p>
            <p className="mb-3 text-note leading-relaxed text-ink-muted">
              {t("assistant.sourceLibrary.separation")}
            </p>
            <div role="radiogroup" aria-labelledby={groupLabel}>
              <div className="grid grid-cols-1 gap-3 min-[480px]:grid-cols-2 min-[720px]:grid-cols-3 min-[1040px]:grid-cols-4">
                {/* Auto is the first card of the grid, the same size as a
                style: a choice among styles, not a banner above them. */}
                <div
                  className={cn(
                    "relative flex min-w-0 flex-col overflow-hidden rounded-list border bg-card transition-colors motion-reduce:transition-none",
                    choice === "auto"
                      ? "border-brand ring-1 ring-brand/50"
                      : "border-line hover:border-ink-faint",
                  )}
                >
                  <button
                    type="button"
                    role="radio"
                    aria-label={t("assistant.presentation.auto")}
                    aria-describedby={autoDescription}
                    aria-checked={choice === "auto"}
                    tabIndex={focusedChoice === "auto" ? 0 : -1}
                    ref={radioRef("auto")}
                    onClick={() => setChoice("auto")}
                    onKeyDown={(event) => move(event, "auto")}
                    className="flex w-full flex-1 flex-col text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand"
                  >
                    <span
                      aria-hidden
                      className="flex aspect-video w-full items-center justify-center bg-brand-soft"
                    >
                      <Sparkles className="h-8 w-8 text-brand-ink" />
                    </span>
                    <span className="block border-t border-line-2 px-3 pb-2.5 pt-2">
                      <span className="flex items-center gap-2">
                        <span className="min-w-0 flex-1 break-words text-ui font-medium text-ink">
                          {t("assistant.presentation.auto")}
                        </span>
                        {choice === "auto" && (
                          <Check
                            aria-hidden
                            className="h-4 w-4 shrink-0 text-brand-ink"
                          />
                        )}
                      </span>
                      <span
                        id={autoDescription}
                        className="mt-1 line-clamp-2 block text-note leading-relaxed text-ink-2"
                      >
                        {t("assistant.presentation.autoDescription")}
                      </span>
                    </span>
                  </button>
                </div>
                {styles.map((style) => {
                  const name = presentationStyleName(style.id, t, language);
                  const selected = choice === style.id;
                  return (
                    <div
                      key={style.id}
                      className={cn(
                        "group relative flex min-w-0 flex-col overflow-hidden rounded-list border bg-card transition-colors motion-reduce:transition-none",
                        selected
                          ? "border-brand ring-1 ring-brand/50"
                          : "border-line hover:border-ink-faint",
                      )}
                    >
                      <button
                        type="button"
                        role="radio"
                        aria-label={name}
                        aria-describedby={`${groupLabel}-${style.id}-description`}
                        aria-checked={selected}
                        tabIndex={focusedChoice === style.id ? 0 : -1}
                        ref={radioRef(style.id)}
                        onClick={() => setChoice(style.id)}
                        onKeyDown={(event) => move(event, style.id)}
                        className="flex w-full flex-1 flex-col text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand"
                      >
                        <CoverImage
                          src={style.cover}
                          alt={t("assistant.presentation.sampleAlt", {
                            name,
                            page: t("assistant.presentation.cover"),
                          })}
                        />
                        <span className="block border-t border-line-2 px-3 pb-2.5 pt-2">
                          <span className="flex items-center gap-2">
                            <span className="min-w-0 flex-1 break-words text-ui font-medium text-ink">
                              {name}
                            </span>
                            {selected && (
                              <Check
                                aria-hidden
                                className="h-4 w-4 shrink-0 text-brand-ink"
                              />
                            )}
                          </span>
                          <span
                            id={`${groupLabel}-${style.id}-description`}
                            className="block"
                          >
                            <span className="mt-1 flex items-start gap-2">
                              <span className="min-w-0 flex-1 text-note text-ink-muted">
                                {presentationStyleScenarios(style, t)}
                              </span>
                              <span className="shrink-0 rounded-sm bg-pill px-1.5 py-0.5 text-micro text-ink-muted">
                                {t(
                                  style.scheme
                                    ? `assistant.presentation.schemes.${style.scheme}`
                                    : "assistant.presentation.builtIn",
                                )}
                              </span>
                            </span>
                            <span className="mt-1 line-clamp-2 text-note leading-relaxed text-ink-2">
                              {presentationStyleIntro(style, t, language)}
                            </span>
                          </span>
                        </span>
                      </button>
                      <div className="border-t border-line-2 p-2">
                        <Button
                          variant="secondary"
                          size="sm"
                          aria-label={t("assistant.presentation.preview", {
                            name,
                          })}
                          className="w-full text-note"
                          onClick={(event) => {
                            previewReturn.current = event.currentTarget;
                            setPreview(style);
                          }}
                        >
                          <Eye aria-hidden className="h-3.5 w-3.5 shrink-0" />
                          {t("assistant.presentation.previewAction")}
                        </Button>
                      </div>
                    </div>
                  );
                })}
              </div>
              {styles.length === 0 && (
                <p
                  role="status"
                  className="py-8 text-center text-ctl text-ink-muted"
                >
                  {t("assistant.presentation.empty")}
                </p>
              )}
            </div>
          </div>
          <DialogFooter className="flex-wrap sm:justify-between">
            <div className="min-w-0 flex-1 text-note leading-relaxed text-ink-muted">
              <p id={selectionLabel} className="mb-1 font-medium text-ink-2">
                {t("assistant.presentation.selected", {
                  name: presentationStyleName(choice, t, language),
                })}
              </p>
              <p>{t("assistant.presentation.sampleNote")}</p>
            </div>
            <div className="flex shrink-0 items-center gap-2">
              <Button variant="secondary" onClick={() => onOpenChange(false)}>
                {t("assistant.presentation.cancel")}
              </Button>
              <Button
                aria-describedby={selectionLabel}
                onClick={() => finish(choice)}
              >
                {t("assistant.presentation.apply")}
              </Button>
            </div>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <PresentationStylePreview
        open={open && preview !== null}
        style={preview}
        source={preview ? (presentationSource(preview.id) ?? null) : null}
        name={preview ? presentationStyleName(preview.id, t, language) : ""}
        onClose={() => setPreview(null)}
        onUse={() => preview && finish(preview.id)}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          if (outerOpen.current)
            (previewReturn.current?.isConnected
              ? previewReturn.current
              : search.current
            )?.focus();
        }}
      />
    </>
  );
}
