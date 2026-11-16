import { useLayoutEffect, useRef, useState, type KeyboardEvent } from "react";
import { ChevronLeft, ChevronRight, RotateCcw } from "lucide-react";
import { settingsApi } from "@/lib/api/settings";
import { useTranslation } from "react-i18next";
import type { PresentationSource } from "@/lib/assistantPresentationSources";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

function SourceImage({
  src,
  alt,
  onRetry,
}: {
  src: string | null;
  alt: string;
  onRetry: () => void;
}) {
  const { t } = useTranslation();
  const [failed, setFailed] = useState(!src);
  const [attempt, setAttempt] = useState(0);
  return failed ? (
    <div className="space-y-3 p-4 text-center text-note text-ink-muted">
      <p role="status">{t("assistant.sourceLibrary.unavailable")}</p>
      {src && (
        <Button
          variant="secondary"
          onClick={() => {
            setFailed(false);
            setAttempt((value) => value + 1);
            onRetry();
          }}
        >
          <RotateCcw aria-hidden className="h-4 w-4" />
          {t("assistant.sourceLibrary.retry")}
        </Button>
      )}
    </div>
  ) : (
    <img
      key={attempt}
      src={src || undefined}
      alt={alt}
      width={1920}
      height={1080}
      decoding="async"
      className="h-full min-h-0 w-full object-contain"
      onError={() => setFailed(true)}
    />
  );
}

/**
 * The upstream deck's own pages, as one view of the style preview. Public
 * static pages only: no HTML execution, IPC query or selection.
 */
export function PresentationSourcePreview({
  source,
}: {
  source: PresentationSource;
}) {
  const { t } = useTranslation();
  const [index, setIndex] = useState(0);
  const [linkFailed, setLinkFailed] = useState(false);
  const viewport = useRef<HTMLElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const previous = useRef<HTMLButtonElement>(null);
  const next = useRef<HTMLButtonElement>(null);
  const thumbs = useRef(new Map<number, HTMLButtonElement>());
  const focus = useRef<"thumbnail" | "viewport" | null>(null);
  const pages = source.pages;
  const current = pages[index];
  useLayoutEffect(() => {
    const thumbnail = thumbs.current.get(index);
    const strip = navigation.current;
    if (thumbnail && strip) {
      const box = strip.getBoundingClientRect(),
        item = thumbnail.getBoundingClientRect();
      if (box.width > 0 && box.height > 0) {
        if (item.top < box.top) strip.scrollTop -= box.top - item.top;
        else if (item.bottom > box.bottom)
          strip.scrollTop += item.bottom - box.bottom;
        if (item.left < box.left) strip.scrollLeft -= box.left - item.left;
        else if (item.right > box.right)
          strip.scrollLeft += item.right - box.right;
      }
    }
    if (focus.current === "thumbnail")
      thumbnail?.focus({ preventScroll: true });
    if (focus.current === "viewport")
      viewport.current?.focus({ preventScroll: true });
    focus.current = null;
  }, [index]);
  const move = (requested: number, thumbnail = false) => {
    if (!pages.length) return;
    const target = Math.max(0, Math.min(pages.length - 1, requested));
    if (target === index) return;
    focus.current = thumbnail
      ? "thumbnail"
      : (target === 0 && document.activeElement === previous.current) ||
          (target === pages.length - 1 &&
            document.activeElement === next.current)
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
      event.nativeEvent.isComposing ||
      (event.target instanceof HTMLElement &&
        event.target.closest(
          'a,summary,input,textarea,select,[contenteditable]:not([contenteditable="false"])',
        ))
    )
      return;
    const target =
      event.key === "Home"
        ? 0
        : event.key === "End"
          ? pages.length - 1
          : ["ArrowLeft", ...(thumbnail ? ["ArrowUp"] : [])].includes(event.key)
            ? index - 1
            : ["ArrowRight", ...(thumbnail ? ["ArrowDown"] : [])].includes(
                  event.key,
                )
              ? index + 1
              : null;
    if (target === null) return;
    event.preventDefault();
    if (thumbnail) event.stopPropagation();
    move(target, thumbnail);
  };
  const link = (name: string, url: string) => (
    <a
      href={url}
      target="_blank"
      rel="noopener noreferrer"
      className="underline underline-offset-2"
      onClick={(event) => {
        event.preventDefault();
        void settingsApi.openExternal(url).catch(() => setLinkFailed(true));
      }}
    >
      {name}
    </a>
  );
  return (
    <div
      className="flex min-h-0 flex-1 flex-col"
      onKeyDown={(event) => keyPage(event)}
    >
      <div className="max-h-28 shrink-0 space-y-1 overflow-auto border-y border-line px-[18px] py-2 text-note text-ink-muted">
        <p>{t("assistant.sourceLibrary.description")}</p>
        <p>
          {t(
            source.kind === "builtin-master"
              ? "assistant.sourceLibrary.builtinSummary"
              : "assistant.sourceLibrary.sourceSummary",
            {
              count:
                source.kind === "builtin-master"
                  ? source.layouts.length
                  : source.pageCount,
            },
          )}{" "}
          ·{" "}
          {source.nativeStatus === "builtin"
            ? t("assistant.sourceLibrary.builtinAdaptation")
            : source.nativeCounts
              ? t("assistant.presentation.mappingCounts", {
                  ...source.nativeCounts,
                })
              : t("assistant.sourceLibrary.partial")}
        </p>
        {source.kind !== "builtin-master" && (
          <p>{t("assistant.sourceLibrary.fonts")}</p>
        )}
        {source.dependencies.map((dependency) => (
          <p key={`${dependency.name} ${dependency.version}`}>
            {t("assistant.sourceLibrary.dependency", dependency)}
          </p>
        ))}
        {source.findings.map((finding) => (
          <p role="alert" key={finding.code} className="text-warn">
            {t(
              finding.code === "original-card-text-clipped"
                ? "assistant.sourceLibrary.cardClipped"
                : "assistant.sourceLibrary.footerClipped",
              { pages: finding.pages.join(", ") },
            )}
          </p>
        ))}
        <details>
          <summary className="cursor-pointer">
            {t("assistant.sourceLibrary.licenses")}
          </summary>
          <div className="space-y-1 py-1">
            {source.sourceUrl && (
              <p>
                {link(
                  `${source.sourceName} · ${source.revision?.slice(0, 12)}`,
                  source.sourceUrl,
                )}
              </p>
            )}
            {source.licenses.map((license) => (
              <p key={license.url}>{link(license.name, license.url)}</p>
            ))}
            <p>{t("assistant.sourceLibrary.licenseBoundary")}</p>
            {linkFailed && (
              <p role="status">{t("assistant.sourceLibrary.linkFailed")}</p>
            )}
          </div>
        </details>
      </div>
      <div className="grid min-h-0 flex-1 grid-rows-[minmax(0,1fr)_86px] gap-3 px-[18px] py-3 min-[640px]:grid-cols-[112px_minmax(0,1fr)] min-[640px]:grid-rows-1">
        <nav
          ref={navigation}
          aria-label={t("assistant.sourceLibrary.pages")}
          className="row-start-2 min-h-0 min-w-0 overflow-auto min-[640px]:row-start-1"
        >
          <ol role="list" className="flex gap-2 min-[640px]:flex-col">
            {pages.map((page, position) => (
              <li
                key={page.number}
                className="w-28 shrink-0 min-[640px]:w-full"
              >
                <button
                  type="button"
                  ref={(node) => {
                    if (node) thumbs.current.set(position, node);
                    else thumbs.current.delete(position);
                  }}
                  aria-label={t("assistant.sourceLibrary.pageLabel", {
                    page: page.number,
                    title: page.label,
                  })}
                  aria-current={index === position ? "page" : undefined}
                  tabIndex={index === position ? 0 : -1}
                  onClick={() => move(position, true)}
                  onKeyDown={(event) => keyPage(event, true)}
                  className={cn(
                    "block w-full overflow-hidden rounded-ctl border text-left outline-none focus-visible:ring-2 focus-visible:ring-brand",
                    index === position
                      ? "border-brand bg-brand-soft ring-1 ring-brand/50"
                      : "border-line bg-card",
                  )}
                >
                  {page.src ? (
                    <img
                      src={page.src}
                      alt=""
                      aria-hidden
                      width={1920}
                      height={1080}
                      loading="lazy"
                      className="aspect-video w-full object-contain"
                    />
                  ) : (
                    <span
                      aria-hidden
                      className="block aspect-video bg-card-2"
                    />
                  )}
                  <span className="block px-1.5 py-1 text-center text-note text-ink-muted">
                    {page.number}
                  </span>
                </button>
              </li>
            ))}
          </ol>
        </nav>
        <figure
          ref={viewport}
          tabIndex={0}
          data-presentation-viewport=""
          aria-label={
            current
              ? t("assistant.sourceLibrary.pageLabel", {
                  page: current.number,
                  title: current.label,
                })
              : t("assistant.sourceLibrary.builtinTitle")
          }
          className="row-start-1 flex min-h-0 min-w-0 items-center justify-center overflow-hidden rounded-ctl border border-line bg-card-2 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand min-[640px]:col-start-2"
        >
          {current ? (
            <SourceImage
              key={current.src || current.number}
              src={current.src}
              alt={t("assistant.sourceLibrary.pageLabel", {
                page: current.number,
                title: current.label,
              })}
              onRetry={() => viewport.current?.focus()}
            />
          ) : (
            <p className="max-w-lg p-5 text-center text-ctl text-ink-muted">
              {t("assistant.sourceLibrary.builtinEmpty", {
                layouts: source.layouts.join(" / "),
              })}
            </p>
          )}
        </figure>
      </div>
      <div className="flex shrink-0 items-center justify-center gap-2 px-[18px] pb-3">
        <Button
          ref={previous}
          variant="secondary"
          size="icon"
          aria-label={t("assistant.sourceLibrary.previous")}
          disabled={index === 0 || !pages.length}
          onClick={() => move(index - 1)}
        >
          <ChevronLeft aria-hidden className="h-4 w-4" />
        </Button>
        <p aria-live="polite" className="text-note text-ink-muted">
          {t("assistant.sourceLibrary.counter", {
            page: pages.length ? index + 1 : 0,
            total: source.pageCount,
          })}
        </p>
        <Button
          ref={next}
          variant="secondary"
          size="icon"
          aria-label={t("assistant.sourceLibrary.next")}
          disabled={!pages.length || index === pages.length - 1}
          onClick={() => move(index + 1)}
        >
          <ChevronRight aria-hidden className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}
