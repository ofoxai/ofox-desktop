import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { ChevronLeft, ChevronRight, Loader2, RotateCcw, X } from "lucide-react";
import { useTranslation } from "react-i18next";
import {
  presentationArchetypeName,
  presentationStyleIntro,
  type PresentationStyle,
} from "@/lib/assistantPresentationStyles";
import type { PresentationSource } from "@/lib/assistantPresentationSources";
import { Button } from "@/components/ui/button";
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
import { PresentationSourcePreview } from "./PresentationSourcePreview";

function PageImage({
  src,
  alt,
  onRetry,
}: {
  src: string;
  alt: string;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  const [state, setState] = useState<"loading" | "ready" | "failed">("loading");
  const [attempt, setAttempt] = useState(0);
  const image = useRef<HTMLImageElement>(null);
  useLayoutEffect(() => {
    if (image.current?.complete && image.current.naturalWidth > 0)
      setState("ready");
  }, [attempt]);
  return (
    <div
      className="relative flex h-full min-h-0 w-full items-center justify-center"
      aria-busy={state === "loading"}
    >
      {state === "failed" ? (
        <div className="max-w-sm space-y-3 p-4 text-center text-note text-ink-muted">
          <p role="status">{t("assistant.presentation.previewUnavailable")}</p>
          <Button
            variant="secondary"
            onClick={() => {
              setState("loading");
              setAttempt((old) => old + 1);
              onRetry();
            }}
          >
            <RotateCcw aria-hidden className="h-3.5 w-3.5" />
            {t("assistant.presentation.retryPage")}
          </Button>
        </div>
      ) : (
        <img
          key={attempt}
          ref={image}
          src={src}
          alt={alt}
          width={1920}
          height={1080}
          decoding="async"
          className="h-full min-h-0 w-full object-contain"
          onLoad={() => setState("ready")}
          onError={() => setState("failed")}
        />
      )}
      {state === "loading" && (
        <p className="pointer-events-none absolute bottom-2 left-1/2 flex -translate-x-1/2 items-center gap-1.5 rounded-ctl bg-card/90 px-2 py-1 text-note text-ink-muted">
          <Loader2
            aria-hidden
            className="h-3 w-3 animate-spin motion-reduce:animate-none"
          />
          {t("assistant.presentation.pageLoading")}
        </p>
      )}
    </div>
  );
}

function Thumbnail({ src, page }: { src: string; page: number }) {
  const [failed, setFailed] = useState(false);
  return failed ? (
    <span
      aria-hidden
      className="flex aspect-video w-full items-center justify-center bg-card-2 text-note text-ink-muted"
    >
      {page}
    </span>
  ) : (
    <img
      src={src}
      alt=""
      aria-hidden
      width={320}
      height={180}
      loading="lazy"
      className="aspect-video w-full object-contain"
      onError={() => setFailed(true)}
    />
  );
}

interface NativePage {
  key: string;
  src: string;
  thumbnail: string;
  /** What the thumbnail and the page viewport are called. */
  label: string;
  /** The short text under a thumbnail; always part of `label`. */
  caption: string;
}

/**
 * Either the built-in master's self-written example, or the pages of a native
 * PPTX that Ofox built for an upstream style. Never the upstream deck itself.
 */
function NativeView({
  style,
  name,
}: {
  style: PresentationStyle;
  name: string;
}) {
  const { t } = useTranslation();
  const [index, setIndex] = useState(0);
  const viewport = useRef<HTMLElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const thumbnails = useRef(new Map<number, HTMLButtonElement>());
  const previous = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const pendingFocus = useRef<"thumbnail" | "viewport" | null>(null);
  const mapped = style.nativePages ?? [];
  const pages: NativePage[] = style.builtIn
    ? style.pages.map((sample, number) => {
        const label = t("assistant.presentation.pageLabel", {
          page: number + 1,
          title: t(`assistant.presentation.pages.${sample.kind}`),
        });
        return {
          key: sample.kind,
          src: sample.src,
          thumbnail: sample.thumbnail,
          label,
          caption: label,
        };
      })
    : mapped.flatMap((sample) => {
        // An unsupported source page has no native page to draw.
        if (!sample.src || sample.status === "unsupported") return [];
        const title = presentationArchetypeName(sample.archetype, t);
        return [
          {
            key: String(sample.sourcePage),
            src: sample.src,
            thumbnail: sample.src,
            label: t("assistant.presentation.nativePageLabel", {
              page: sample.sourcePage,
              title,
              status: t(
                `assistant.presentation.mappingStatus.${sample.status}`,
              ),
            }),
            caption: title,
          },
        ];
      });
  const refused = mapped.filter((sample) => sample.status === "unsupported");
  const page = pages[index];

  useLayoutEffect(() => {
    const thumb = thumbnails.current.get(index);
    const nav = navigation.current;
    // Scroll only this strip. scrollIntoView could move the outer gallery.
    if (thumb && nav) {
      const box = nav.getBoundingClientRect();
      const item = thumb.getBoundingClientRect();
      if (box.height > 0 && box.width > 0) {
        if (item.top < box.top) nav.scrollTop -= box.top - item.top;
        else if (item.bottom > box.bottom)
          nav.scrollTop += item.bottom - box.bottom;
        if (item.left < box.left) nav.scrollLeft -= box.left - item.left;
        else if (item.right > box.right)
          nav.scrollLeft += item.right - box.right;
      }
    }
    if (pendingFocus.current === "thumbnail")
      thumb?.focus({ preventScroll: true });
    else if (pendingFocus.current === "viewport")
      viewport.current?.focus({ preventScroll: true });
    pendingFocus.current = null;
  }, [index]);

  const move = (requested: number, focusThumbnail = false) => {
    const target = Math.max(0, Math.min(pages.length - 1, requested));
    if (target === index || !pages.length) return;
    const active = document.activeElement;
    pendingFocus.current = focusThumbnail
      ? "thumbnail"
      : (target === 0 && active === previous.current) ||
          (target === pages.length - 1 && active === next.current)
        ? "viewport"
        : null;
    setIndex(target);
  };
  const keyPage = (event: KeyboardEvent<HTMLElement>, thumbnail = false) => {
    if (
      event.defaultPrevented ||
      event.altKey ||
      event.ctrlKey ||
      event.metaKey ||
      event.nativeEvent.isComposing
    )
      return;
    const target = event.target;
    if (
      target instanceof HTMLElement &&
      target.closest(
        'input, textarea, select, [contenteditable]:not([contenteditable="false"])',
      )
    )
      return;
    const requested =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? pages.length - 1
          : event.key === "ArrowLeft" || (thumbnail && event.key === "ArrowUp")
            ? index - 1
            : event.key === "ArrowRight" ||
                (thumbnail && event.key === "ArrowDown")
              ? index + 1
              : null;
    if (requested === null) return;
    event.preventDefault();
    if (thumbnail) event.stopPropagation();
    move(requested, thumbnail);
  };
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onKeyDown={(event) => keyPage(event)}
    >
      <div className="max-h-28 shrink-0 space-y-1 overflow-auto border-y border-line px-[18px] py-2 text-note text-ink-muted">
        {style.builtIn ? (
          <p>{t("assistant.presentation.previewDescription")}</p>
        ) : (
          <>
            {pages.length > 0 && (
              <p>{t("assistant.presentation.nativeNotice")}</p>
            )}
            {style.counts && (
              <p className="font-medium text-ink-2">
                {t("assistant.presentation.mappingCounts", { ...style.counts })}
              </p>
            )}
            {refused.length > 0 && (
              <div>
                <p>
                  {t("assistant.presentation.unsupportedTitle", {
                    count: refused.length,
                  })}
                </p>
                <ul
                  role="list"
                  aria-label={t("assistant.presentation.unsupportedTitle", {
                    count: refused.length,
                  })}
                  className="list-disc pl-5"
                >
                  {refused.map((sample) => (
                    <li key={sample.sourcePage}>
                      {t(
                        sample.differences
                          ? "assistant.presentation.unsupportedItem"
                          : "assistant.presentation.nativePageLabel",
                        {
                          page: sample.sourcePage,
                          title: presentationArchetypeName(sample.archetype, t),
                          status: t(
                            "assistant.presentation.mappingStatus.unsupported",
                          ),
                          text: sample.differences,
                        },
                      )}
                    </li>
                  ))}
                </ul>
              </div>
            )}
          </>
        )}
      </div>
      {pages.length === 0 ? (
        <p className="flex min-h-0 flex-1 items-center justify-center p-6 text-center text-ctl text-ink-muted">
          {t("assistant.presentation.nativeUnavailable")}
        </p>
      ) : (
        <>
          <div className="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_88px] gap-3 px-[18px] py-3 min-[640px]:grid-cols-[112px_minmax(0,1fr)] min-[640px]:grid-rows-1">
            <nav
              ref={navigation}
              aria-label={t("assistant.presentation.thumbnails")}
              className="row-start-2 min-h-0 min-w-0 overflow-auto min-[640px]:row-start-1"
            >
              <ol role="list" className="flex gap-2 min-[640px]:flex-col">
                {pages.map((sample, number) => (
                  <li
                    key={sample.key}
                    className="w-28 shrink-0 min-[640px]:w-full"
                  >
                    <button
                      type="button"
                      aria-label={sample.label}
                      aria-current={index === number ? "page" : undefined}
                      tabIndex={index === number ? 0 : -1}
                      ref={(element) => {
                        if (element) thumbnails.current.set(number, element);
                        else thumbnails.current.delete(number);
                      }}
                      onClick={() => move(number, true)}
                      onKeyDown={(event) => keyPage(event, true)}
                      className={cn(
                        "block w-full overflow-hidden rounded-ctl border text-left outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand",
                        index === number
                          ? "border-brand bg-brand-soft ring-1 ring-brand/50"
                          : "border-line bg-card",
                      )}
                    >
                      <Thumbnail
                        key={sample.thumbnail}
                        src={sample.thumbnail}
                        page={number + 1}
                      />
                      <span className="block truncate px-1.5 py-1 text-micro text-ink-2">
                        {sample.caption}
                      </span>
                    </button>
                  </li>
                ))}
              </ol>
            </nav>
            <div className="row-start-1 flex min-h-0 min-w-0 flex-col gap-2 min-[640px]:col-start-2">
              <figure
                ref={viewport}
                tabIndex={-1}
                data-presentation-viewport=""
                aria-label={page?.label}
                className="flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-hidden rounded-ctl border border-line bg-card-2 outline-none"
              >
                {page && (
                  <PageImage
                    key={page.src}
                    src={page.src}
                    alt={t("assistant.presentation.sampleAlt", {
                      name,
                      page: page.label,
                    })}
                    onRetry={() =>
                      viewport.current?.focus({ preventScroll: true })
                    }
                  />
                )}
              </figure>
            </div>
          </div>
          <div className="flex shrink-0 items-center justify-center gap-2 px-[18px] pb-3">
            <Button
              ref={previous}
              variant="secondary"
              aria-label={t("assistant.presentation.previousPage")}
              disabled={index === 0}
              onClick={() => move(index - 1)}
            >
              <ChevronLeft aria-hidden className="h-4 w-4" />
            </Button>
            <p role="status" className="text-note tabular-nums text-ink-2">
              {t("assistant.presentation.pageCounter", {
                page: index + 1,
                total: pages.length,
              })}
            </p>
            <Button
              ref={next}
              variant="secondary"
              aria-label={t("assistant.presentation.nextPage")}
              disabled={index >= pages.length - 1}
              onClick={() => move(index + 1)}
            >
              <ChevronRight aria-hidden className="h-4 w-4" />
            </Button>
          </div>
        </>
      )}
    </div>
  );
}

type PreviewView = "native" | "original";
type PreviewProps = {
  style: PresentationStyle | null;
  /** The style's upstream deck record, when coverage has one. */
  source: PresentationSource | null;
  name: string;
  open: boolean;
  onClose: () => void;
  onUse: () => void;
  onCloseAutoFocus?: (event: Event) => void;
};

/** A keyed open session resets view/page state without copying props into state. */
function PreviewSession({
  style,
  source,
  name,
  onClose,
  onUse,
  onCloseAutoFocus,
}: Omit<PreviewProps, "style" | "open"> & { style: PresentationStyle }) {
  const { t, i18n } = useTranslation();
  const content = useRef<HTMLDivElement>(null);
  const drawable =
    style.builtIn || (style.nativePages ?? []).some((page) => page.src);
  // With nothing native to draw yet, the upstream deck is the useful view.
  const [view, setView] = useState<PreviewView>(
    drawable || !source ? "native" : "original",
  );
  return (
    <DialogContent
      ref={content}
      zIndex="nested"
      className="h-[min(90vh,900px)] w-[calc(100vw-32px)] max-w-[1200px] overflow-hidden"
      onCloseAutoFocus={onCloseAutoFocus}
      onOpenAutoFocus={(event) => {
        event.preventDefault();
        (
          content.current?.querySelector<HTMLElement>(
            "[data-presentation-viewport]",
          ) ?? content.current
        )?.focus();
      }}
    >
      <DialogHeader className="pr-12">
        <DialogTitle>
          {t("assistant.presentation.previewTitle", { name })}
        </DialogTitle>
        <DialogDescription>
          {presentationStyleIntro(style, t, i18n.language)}
        </DialogDescription>
        <Button
          variant="icon"
          aria-label={t("assistant.presentation.closePreview")}
          className="absolute right-3 top-3"
          onClick={onClose}
        >
          <X aria-hidden className="h-4 w-4" />
        </Button>
      </DialogHeader>
      <div className="flex shrink-0 px-[18px] pb-2">
        <SegmentedControl<PreviewView>
          aria-label={t("assistant.presentation.view")}
          value={view}
          onChange={setView}
          options={[
            { value: "native", label: t("assistant.presentation.viewNative") },
            {
              value: "original",
              label: t("assistant.presentation.viewOriginal"),
            },
          ]}
        />
      </div>
      {view === "native" ? (
        <NativeView style={style} name={name} />
      ) : source ? (
        <PresentationSourcePreview source={source} />
      ) : (
        <p className="flex min-h-0 flex-1 items-center justify-center border-t border-line p-6 text-center text-ctl text-ink-muted">
          {t("assistant.sourceLibrary.missing")}
        </p>
      )}
      <DialogFooter>
        <Button variant="secondary" onClick={onClose}>
          {t("assistant.presentation.backToStyles")}
        </Button>
        <Button onClick={onUse}>
          {t("assistant.presentation.usePreview", { name })}
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}

/**
 * Static local pages in two separate views: Ofox's native pages and the
 * upstream original. Browsing never selects a style or runs a model.
 */
export function PresentationStylePreview({
  style,
  source,
  name,
  open,
  onClose,
  onUse,
  onCloseAutoFocus,
}: PreviewProps) {
  return (
    <Dialog
      open={open && style !== null}
      onOpenChange={(next) => {
        if (!next) onClose();
      }}
    >
      {open && style && (
        <PreviewSession
          key={style.id}
          style={style}
          source={source}
          name={name}
          onClose={onClose}
          onUse={onUse}
          onCloseAutoFocus={onCloseAutoFocus}
        />
      )}
    </Dialog>
  );
}
