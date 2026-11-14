import { FoxLoader } from "@/components/ui/fox-loader";
import {
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import {
  ChevronLeft,
  ChevronRight,
  ChevronDown,
  Ellipsis,
  Files,
  Maximize2,
  Minimize2,
  Quote,
  RotateCcw,
  X,
} from "lucide-react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  DropdownMenu,
  DropdownMenuCheckboxItem,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import type { AssistantAttachment } from "@/lib/api/assistant";
import {
  assistantPreviewApi,
  assistantPresentationRevision,
  type FilePageReference,
  type FilePreviewManifest,
  type FilePreviewPage,
  type FilePreviewRequest,
} from "@/lib/api/assistantPreview";
import { cn } from "@/lib/utils";

type Authority = Omit<FilePreviewRequest, "attachmentId">;
type CachedPage = FilePreviewPage & { src: string; scale: 1 | 2 };
type PreviewHeader = (actions?: ReactNode) => ReactNode;

/** The chat remains mounted while its file pane opens, resizes or expands. */
/** A preview that has not arrived yet: the loading fox, centred in its pane. */
function PreviewLoading() {
  const { t } = useTranslation();
  return (
    <FoxLoader
      size="block"
      label={t("assistant.workspace.loading")}
      className="min-h-40 flex-1"
    />
  );
}

export function AssistantFileWorkspace({
  children,
  files,
  authority,
  selectedId,
  expectedVersion,
  onSelect,
  onQuote,
  onPrepare,
  active = true,
}: {
  children: ReactNode;
  files: AssistantAttachment[];
  authority: Authority | null;
  selectedId: string | null;
  expectedVersion?: string;
  onSelect: (id: string | null, expectedVersion?: string) => void;
  onQuote: (reference: FilePageReference) => void;
  onPrepare?: (
    reference: FilePageReference,
    action: "optimize" | "replace-image",
  ) => void;
  active?: boolean;
}) {
  const { t } = useTranslation();
  const root = useRef<HTMLDivElement>(null);
  const opener = useRef<HTMLButtonElement>(null);
  const focusOpener = useRef(false);
  const focusVersion = useRef(false);
  const fileHeading = useRef<HTMLHeadingElement | null>(null);
  // Loading and loaded previews replace their heading; retain its focus only
  // when the user has not moved elsewhere while the manifest was pending.
  const carryHeadingFocus = useRef(false);
  const attachHeading = useCallback((node: HTMLHeadingElement | null) => {
    if (!node)
      carryHeadingFocus.current =
        document.activeElement === fileHeading.current;
    fileHeading.current = node;
    if (node && carryHeadingFocus.current) {
      carryHeadingFocus.current = false;
      node.focus();
    }
  }, []);
  const [ratio, setRatio] = useState(50);
  const [narrow, setNarrow] = useState(false);
  const [tab, setTab] = useState<"chat" | "files">("files");
  const [expanded, setExpanded] = useState(false);
  const id = useId();
  const selected = files.find((file) => file.id === selectedId);
  const open = Boolean(active && authority && selected);
  useLayoutEffect(() => {
    if (!open && focusOpener.current) {
      focusOpener.current = false;
      opener.current?.focus();
    }
  }, [open]);
  useLayoutEffect(() => {
    if (open && focusVersion.current) {
      focusVersion.current = false;
      fileHeading.current?.focus();
    }
  }, [open, selectedId, expectedVersion]);
  useLayoutEffect(() => {
    const node = root.current;
    if (!node) return;
    const measure = () =>
      setNarrow(
        node.getBoundingClientRect().width > 0 &&
          node.getBoundingClientRect().width < 760,
      );
    measure();
    const observer =
      typeof ResizeObserver === "undefined"
        ? null
        : new ResizeObserver(measure);
    observer?.observe(node);
    window.addEventListener("resize", measure);
    return () => {
      observer?.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, []);
  useEffect(() => {
    setTab("files");
    setExpanded(false);
  }, [
    selectedId,
    authority?.sessionId,
    authority?.expectedOwner,
    authority?.expectedGeneration,
  ]);
  const close = () => {
    focusOpener.current = true;
    onSelect(null);
  };
  const header: PreviewHeader = (actions) => (
    <div className="flex h-11 min-w-0 shrink-0 items-center gap-1 border-b border-line px-3">
      <div className="flex min-w-0 flex-1 items-center gap-1">
        <h2
          ref={attachHeading}
          tabIndex={-1}
          className="truncate text-ctl font-medium outline-none"
          title={selected?.name}
        >
          {selected?.name}
        </h2>
        {files.length > 1 && (
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              <Button
                variant="icon"
                aria-label={t("assistant.workspace.switchFile")}
              >
                <ChevronDown aria-hidden className="h-3.5 w-3.5" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="start"
              className="max-h-[50vh] max-w-[min(320px,calc(100vw-32px))] overflow-y-auto"
              onEscapeKeyDown={(event) => event.stopPropagation()}
            >
              {files.map((file) => (
                <DropdownMenuItem
                  key={file.id}
                  onSelect={() => {
                    if (file.id !== selectedId) focusVersion.current = true;
                    onSelect(file.id);
                  }}
                  title={file.name}
                  aria-current={file.id === selectedId ? "true" : undefined}
                  className="truncate"
                >
                  {file.name}
                </DropdownMenuItem>
              ))}
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </div>
      {actions}
      {!narrow && (
        <Button
          variant="icon"
          aria-label={t(
            expanded
              ? "assistant.workspace.restore"
              : "assistant.workspace.expand",
          )}
          onClick={() => setExpanded((old) => !old)}
        >
          {expanded ? (
            <Minimize2 aria-hidden className="h-4 w-4" />
          ) : (
            <Maximize2 aria-hidden className="h-4 w-4" />
          )}
        </Button>
      )}
      <Button
        variant="icon"
        aria-label={t("assistant.workspace.close")}
        onClick={close}
      >
        <X aria-hidden className="h-4 w-4" />
      </Button>
    </div>
  );
  return (
    <div
      ref={root}
      className="relative flex min-h-0 min-w-0 flex-1 flex-col"
      data-file-workspace={open ? "open" : "closed"}
    >
      {files.length > 0 && (!open || narrow) && (
        <div className="flex shrink-0 items-center justify-end gap-2 border-b border-line px-2 py-1">
          {narrow && open && (
            <div
              className="mr-auto flex gap-1"
              role="group"
              aria-label={t("assistant.workspace.view")}
            >
              <Button
                variant={tab === "chat" ? "secondary" : "ghost"}
                aria-pressed={tab === "chat"}
                onClick={() => setTab("chat")}
              >
                {t("assistant.chat")}
              </Button>
              <Button
                variant={tab === "files" ? "secondary" : "ghost"}
                aria-pressed={tab === "files"}
                onClick={() => setTab("files")}
              >
                {t("assistant.workspace.files")}
              </Button>
            </div>
          )}
          {!open && (
            <Button
              ref={opener}
              variant="ghost"
              onClick={() => onSelect(files[files.length - 1].id)}
              aria-controls={id}
              aria-expanded={false}
            >
              <Files aria-hidden className="h-4 w-4" />
              {t("assistant.workspace.open", { count: files.length })}
            </Button>
          )}
        </div>
      )}
      <div className="flex min-h-0 min-w-0 flex-1 overflow-hidden">
        <div
          className={cn(
            "flex min-h-0 min-w-0 flex-1 flex-col",
            open && ((narrow && tab === "files") || expanded) && "hidden",
          )}
          style={
            open && !narrow && !expanded
              ? { flexBasis: `${100 - ratio}%` }
              : undefined
          }
        >
          {children}
        </div>
        {open && !narrow && !expanded && (
          <div
            role="separator"
            tabIndex={0}
            aria-label={t("assistant.workspace.resize")}
            aria-orientation="vertical"
            aria-valuemin={35}
            aria-valuemax={65}
            aria-valuenow={ratio}
            aria-controls={id}
            className="w-2 shrink-0 cursor-col-resize touch-none border-x border-line bg-card hover:bg-pill focus-visible:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand"
            onKeyDown={(event) => {
              if (
                !["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)
              )
                return;
              event.preventDefault();
              setRatio((old) =>
                event.key === "Home"
                  ? 35
                  : event.key === "End"
                    ? 65
                    : Math.max(
                        35,
                        Math.min(
                          65,
                          old + (event.key === "ArrowLeft" ? 2 : -2),
                        ),
                      ),
              );
            }}
            onPointerDown={(event) => {
              event.currentTarget.setPointerCapture(event.pointerId);
            }}
            onPointerMove={(event) => {
              if (!event.currentTarget.hasPointerCapture(event.pointerId))
                return;
              const bounds = root.current?.getBoundingClientRect();
              if (bounds?.width)
                setRatio(
                  Math.max(
                    35,
                    Math.min(
                      65,
                      (100 * (bounds.right - event.clientX)) / bounds.width,
                    ),
                  ),
                );
            }}
            onPointerUp={(event) => {
              if (event.currentTarget.hasPointerCapture(event.pointerId))
                event.currentTarget.releasePointerCapture(event.pointerId);
            }}
          />
        )}
        {open && authority && selected && (
          <aside
            id={id}
            aria-label={t("assistant.workspace.title")}
            className={cn(
              "flex min-h-0 min-w-0 flex-1 flex-col bg-card-2",
              narrow && tab === "chat" && "hidden",
            )}
            style={
              !narrow && !expanded ? { flexBasis: `${ratio}%` } : undefined
            }
            onKeyDown={(event) => {
              if (event.key !== "Escape" || event.defaultPrevented) return;
              event.preventDefault();
              event.stopPropagation();
              close();
            }}
          >
            <ArtifactPreview
              key={`${authority.expectedOwner}:${authority.expectedGeneration}:${authority.sessionId}:${selected.id}:${selected.sizeBytes}:${expectedVersion ?? "latest"}`}
              request={{ ...authority, attachmentId: selected.id }}
              expectedVersion={expectedVersion}
              header={header}
              revisionControl={
                <RevisionSwitch
                  key={`${authority.expectedOwner}:${authority.expectedGeneration}:${authority.sessionId}:${selected.id}:${expectedVersion ?? "latest"}`}
                  file={selected}
                  files={files}
                  authority={authority}
                  onSelect={(id, version) => {
                    focusVersion.current = true;
                    onSelect(id, version);
                  }}
                />
              }
              initialPage={
                assistantPresentationRevision(selected)?.pageNumbers[0]
              }
              onQuote={(reference) => {
                setTab("chat");
                setExpanded(false);
                onQuote(reference);
              }}
              onPrepare={
                onPrepare
                  ? (reference, action) => {
                      setTab("chat");
                      setExpanded(false);
                      onPrepare(reference, action);
                    }
                  : undefined
              }
            />
          </aside>
        )}
      </div>
    </div>
  );
}

function pageRanges(pages: number[]) {
  pages = [...pages].sort((a, b) => a - b);
  const ranges: string[] = [];
  for (let start = 0; start < pages.length; ) {
    let end = start;
    while (end + 1 < pages.length && pages[end + 1] === pages[end] + 1)
      end += 1;
    ranges.push(
      start === end ? String(pages[start]) : `${pages[start]}–${pages[end]}`,
    );
    start = end + 1;
  }
  return ranges.join(", ");
}

/** A version switch; it never writes, deletes or regenerates either artifact. */
function RevisionSwitch({
  file,
  files,
  authority,
  onSelect,
}: {
  file: AssistantAttachment;
  files: AssistantAttachment[];
  authority: Authority;
  onSelect: (id: string | null, expectedVersion?: string) => void;
}) {
  const { t } = useTranslation();
  const revision = assistantPresentationRevision(file);
  const parent = revision
    ? files.find(
        (candidate) =>
          candidate.id === revision.parentAttachmentId &&
          candidate.status !== "missing" &&
          candidate.mimeType === file.mimeType,
      )
    : undefined;
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<"changed" | "unavailable" | null>(null);
  const mounted = useRef(true);
  const button = useRef<HTMLButtonElement>(null);
  const info = useRef<HTMLParagraphElement>(null);
  const latest = useRef({ parent, revision });
  latest.current = { parent, revision };
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  if (!revision) return null;
  const undo = async () => {
    if (!parent || checking) return;
    const keepFocus = document.activeElement === button.current;
    if (keepFocus) info.current?.focus();
    setChecking(true);
    setError(null);
    try {
      const found = await assistantPreviewApi.manifest({
        ...authority,
        attachmentId: parent.id,
      });
      if (!mounted.current) return;
      if (
        !latest.current.parent ||
        latest.current.parent.id !== parent.id ||
        latest.current.revision?.parentVersion !== revision.parentVersion
      ) {
        setError("unavailable");
        return;
      }
      if (found.version !== revision.parentVersion) {
        setError("changed");
        return;
      }
      onSelect(parent.id, revision.parentVersion);
    } catch {
      if (mounted.current) setError("unavailable");
    } finally {
      if (mounted.current) {
        setChecking(false);
        if (keepFocus && document.activeElement === info.current)
          requestAnimationFrame(() => {
            if (mounted.current && document.activeElement === info.current)
              button.current?.focus();
          });
      }
    }
  };
  return (
    <div className="shrink-0 border-b border-line px-2 py-1 text-note text-ink-muted">
      <div className="flex min-w-0 items-center gap-2">
        <p
          ref={info}
          tabIndex={-1}
          className="min-w-0 flex-1 truncate outline-none"
          title={t("assistant.workspace.versionSwitchOnly")}
        >
          {t("assistant.workspace.sourceVersion", {
            version: revision.parentVersion.slice(0, 12),
          })}
          {" · "}
          <span>
            {t("assistant.workspace.changedPages", {
              pages: pageRanges(revision.pageNumbers),
            })}
          </span>
        </p>
        <Button
          ref={button}
          variant="icon"
          disabled={!parent || checking}
          title={t("assistant.workspace.versionSwitchOnly")}
          aria-label={t(
            checking
              ? "assistant.workspace.checkingVersion"
              : "assistant.workspace.previousVersion",
          )}
          onClick={() => void undo()}
        >
          <RotateCcw aria-hidden className="h-3.5 w-3.5" />
        </Button>
      </div>
      {!parent && <p>{t("assistant.workspace.previousUnavailable")}</p>}
      {error && (
        <p role="alert" className="text-warn">
          {t(
            error === "changed"
              ? "assistant.errors.previewVersionChanged"
              : "assistant.workspace.unavailable",
          )}
        </p>
      )}
    </div>
  );
}

function ArtifactPreview({
  request,
  expectedVersion,
  initialPage,
  onQuote,
  onPrepare,
  header,
  revisionControl,
}: {
  header: PreviewHeader;
  revisionControl: ReactNode;
  request: FilePreviewRequest;
  expectedVersion?: string;
  initialPage?: number;
  onQuote: (reference: FilePageReference) => void;
  onPrepare?: (
    reference: FilePageReference,
    action: "optimize" | "replace-image",
  ) => void;
}) {
  const { t } = useTranslation();
  const [manifest, setManifest] = useState<FilePreviewManifest | null>(null);
  const [failed, setFailed] = useState(false);
  const [versionChanged, setVersionChanged] = useState(false);
  const [attempt, setAttempt] = useState(0);
  useEffect(() => {
    let current = true;
    setManifest(null);
    setFailed(false);
    setVersionChanged(false);
    void assistantPreviewApi.manifest(request).then(
      (value) => {
        if (current) {
          if (expectedVersion && value.version !== expectedVersion) {
            setFailed(true);
            setVersionChanged(true);
          } else setManifest(value);
        }
      },
      () => {
        if (current) setFailed(true);
      },
    );
    return () => {
      current = false;
    };
  }, [
    request.sessionId,
    request.expectedOwner,
    request.expectedGeneration,
    request.attachmentId,
    attempt,
    expectedVersion,
  ]);
  if (!manifest)
    return (
      <>
        {header()}
        {versionChanged || failed ? (
          <div className="space-y-3 p-4 text-note text-ink-muted" role="status">
            {t(
              versionChanged
                ? "assistant.errors.previewVersionChanged"
                : "assistant.workspace.unavailable",
            )}
            {failed && (
              <Button
                variant="secondary"
                onClick={() => setAttempt((old) => old + 1)}
              >
                {t("assistant.workspace.retry")}
              </Button>
            )}
          </div>
        ) : (
          <PreviewLoading />
        )}
      </>
    );
  const quote = (page: number | null) =>
    onQuote({
      attachmentId: manifest.attachmentId,
      version: manifest.version,
      name: manifest.name,
      page,
    });
  const prepare = onPrepare
    ? (page: number, action: "optimize" | "replace-image") =>
        onPrepare(
          {
            attachmentId: manifest.attachmentId,
            version: manifest.version,
            name: manifest.name,
            page,
          },
          action,
        )
    : undefined;
  return (
    <>
      {manifest.kind === "pptx" ? (
        <PptxPreview
          key={manifest.version}
          request={request}
          manifest={manifest}
          header={header}
          revisionControl={revisionControl}
          initialPage={initialPage}
          onQuote={quote}
          onPrepare={prepare}
          onReload={() => setAttempt((old) => old + 1)}
        />
      ) : (
        <HtmlPreview
          key={manifest.version}
          request={request}
          manifest={manifest}
          header={header}
          revisionControl={revisionControl}
          onQuote={() => quote(null)}
          onReload={() => setAttempt((old) => old + 1)}
        />
      )}
    </>
  );
}

/** Secondary controls and technical details stay behind one header menu. */
function PreviewActions({
  manifest,
  onReload,
  notices,
  revisionControl,
  onQuote,
  quoteDisabled,
  onPrepare,
  children,
  warning = false,
}: {
  manifest: FilePreviewManifest;
  onReload: () => void;
  notices: string[];
  revisionControl: ReactNode;
  onQuote?: () => void;
  quoteDisabled?: boolean;
  onPrepare?: (action: "optimize" | "replace-image") => void;
  children?: ReactNode;
  warning?: boolean;
}) {
  const { t } = useTranslation();
  const more = useRef<HTMLButtonElement>(null);
  const acted = useRef(false);
  const openingDetails = useRef(false);
  const [detailsOpen, setDetailsOpen] = useState(false);
  const action = (run: () => void) => {
    acted.current = true;
    run();
  };
  return (
    <>
      <DropdownMenu modal={false}>
        <DropdownMenuTrigger asChild>
          <Button
            ref={more}
            variant="icon"
            aria-label={t("assistant.workspace.moreActions")}
            title={t("assistant.workspace.moreActions")}
            className="relative"
          >
            <Ellipsis aria-hidden className="h-4 w-4" />
            {warning && (
              <span
                aria-hidden
                className="absolute right-1 top-1 h-1.5 w-1.5 rounded-full bg-warn"
              />
            )}
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="end"
          className="max-h-[70vh] max-w-[min(320px,calc(100vw-32px))] overflow-y-auto"
          onEscapeKeyDown={(event) => event.stopPropagation()}
          onCloseAutoFocus={(event) => {
            if (openingDetails.current) {
              event.preventDefault();
              openingDetails.current = false;
            }
            if (acted.current) {
              const focused = document.activeElement;
              if (
                focused instanceof HTMLElement &&
                focused !== document.body &&
                focused.isConnected &&
                !focused.closest('[role="menu"]')
              )
                event.preventDefault();
              acted.current = false;
            }
          }}
        >
          {onPrepare && (
            <>
              <DropdownMenuLabel className="max-w-[280px] whitespace-normal text-note font-normal text-ink-muted">
                {t("assistant.workspace.prepareOnly")}
              </DropdownMenuLabel>
              <DropdownMenuItem
                disabled={quoteDisabled}
                onSelect={() => action(() => onPrepare("optimize"))}
              >
                {t("assistant.workspace.optimizePage")}
              </DropdownMenuItem>
              <DropdownMenuItem
                disabled={quoteDisabled}
                onSelect={() => action(() => onPrepare("replace-image"))}
              >
                {t("assistant.workspace.replaceImage")}
              </DropdownMenuItem>
            </>
          )}
          {onQuote && (
            <DropdownMenuItem
              disabled={quoteDisabled}
              onSelect={() => action(onQuote)}
            >
              <Quote aria-hidden className="h-3.5 w-3.5" />
              {t(
                manifest.kind === "pptx"
                  ? "assistant.workspace.quotePage"
                  : "assistant.workspace.quoteFile",
              )}
            </DropdownMenuItem>
          )}
          {children}
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={onReload}>
            {t("assistant.workspace.reload")}
          </DropdownMenuItem>
          <DropdownMenuItem
            onSelect={() => {
              openingDetails.current = true;
              setDetailsOpen(true);
            }}
          >
            {t("assistant.workspace.previewDetails")}
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
      <Dialog open={detailsOpen} onOpenChange={setDetailsOpen}>
        <DialogContent
          className="gap-3 p-4"
          onEscapeKeyDown={(event) => event.stopPropagation()}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            more.current?.focus();
          }}
        >
          <DialogHeader className="flex-row items-start gap-3">
            <div className="min-w-0 flex-1 space-y-1">
              <DialogTitle>
                {t("assistant.workspace.previewDetails")}
              </DialogTitle>
              <DialogDescription className="break-all">
                {t("assistant.workspace.version", {
                  version: manifest.version,
                })}
              </DialogDescription>
            </div>
            <DialogClose asChild>
              <Button variant="icon" aria-label={t("common.close")}>
                <X aria-hidden className="h-4 w-4" />
              </Button>
            </DialogClose>
          </DialogHeader>
          {notices.map((notice, index) => (
            <p
              key={index}
              className={cn(
                "break-words text-note text-ink-muted",
                warning && "text-warn",
              )}
            >
              {notice}
            </p>
          ))}
          {revisionControl}
        </DialogContent>
      </Dialog>
    </>
  );
}

function PptxPreview({
  request,
  manifest,
  initialPage,
  onQuote,
  onPrepare,
  onReload,
  header,
  revisionControl,
}: {
  header: PreviewHeader;
  revisionControl: ReactNode;
  request: FilePreviewRequest;
  manifest: FilePreviewManifest;
  initialPage?: number;
  onQuote: (page: number) => void;
  onPrepare?: (page: number, action: "optimize" | "replace-image") => void;
  onReload: () => void;
}) {
  const { t } = useTranslation();
  const lastPage = Math.min(
    manifest.totalPages,
    manifest.limits.maxPage ?? manifest.totalPages,
  );
  const validInitial =
    initialPage !== undefined &&
    Number.isInteger(initialPage) &&
    initialPage >= 1 &&
    initialPage <= lastPage;
  const [page, setPage] = useState(() => (validInitial ? initialPage : 1));
  const [pageInput, setPageInput] = useState(() =>
    String(validInitial ? initialPage : 1),
  );
  const [zoom, setZoom] = useState<"fit" | "100" | "200">("fit");
  const [result, setResult] = useState<CachedPage | null>(null);
  const [failed, setFailed] = useState(false);
  const [attempt, setAttempt] = useState(0);
  const cache = useRef(new Map<string, CachedPage>());
  const viewport = useRef<HTMLDivElement>(null);
  const navigation = useRef<HTMLElement>(null);
  const [showThumbnails, setShowThumbnails] = useState(false);
  const [jumpOpen, setJumpOpen] = useState(false);
  const jumped = useRef(false);
  const scale: 1 | 2 = zoom === "200" ? 2 : 1;
  useLayoutEffect(() => {
    const nav = navigation.current;
    const button = nav?.querySelector<HTMLButtonElement>(
      '[aria-current="page"]',
    );
    if (!nav || !button) return;
    if (button.offsetLeft < nav.scrollLeft) nav.scrollLeft = button.offsetLeft;
    else if (
      button.offsetLeft + button.offsetWidth >
      nav.scrollLeft + nav.clientWidth
    )
      nav.scrollLeft = button.offsetLeft + button.offsetWidth - nav.clientWidth;
  }, [page, lastPage, showThumbnails]);
  const imageTicket = `${page}:${scale}:${attempt}`;
  const imageTicketRef = useRef(imageTicket);
  imageTicketRef.current = imageTicket;
  useEffect(() => {
    if (lastPage === 0) return;
    let current = true;
    const key = `${page}:${scale}`;
    setFailed(false);
    const cached = cache.current.get(key);
    setResult(cached ?? null);
    if (!cached)
      void assistantPreviewApi
        .page({ ...request, version: manifest.version, page, scale })
        .then(
          (value) => {
            if (!current) return;
            const next = {
              ...value,
              scale,
              src: `data:image/jpeg;base64,${value.dataBase64}`,
            };
            cache.current.set(key, next);
            if (cache.current.size > 12)
              cache.current.delete(cache.current.keys().next().value!);
            setResult(next);
          },
          () => {
            if (current) setFailed(true);
          },
        );
    return () => {
      current = false;
    };
  }, [
    request.sessionId,
    request.expectedOwner,
    request.expectedGeneration,
    request.attachmentId,
    manifest.version,
    page,
    scale,
    attempt,
    lastPage,
  ]);
  const move = (number: number, focus = false) => {
    const next = Math.max(1, Math.min(lastPage, number));
    setPage(next);
    setPageInput(String(next));
    if (focus || next === 1 || next === lastPage) viewport.current?.focus();
  };
  const keys = (event: KeyboardEvent<HTMLElement>) => {
    if ((event.target as HTMLElement).closest("input, [role=combobox]")) return;
    const next =
      event.key === "ArrowRight"
        ? page + 1
        : event.key === "ArrowLeft"
          ? page - 1
          : event.key === "Home"
            ? 1
            : event.key === "End"
              ? lastPage
              : null;
    if (next === null) return;
    event.preventDefault();
    move(next, true);
  };
  // Late responses can never label a previous page as the current one.
  const visible =
    result?.page === page && result.scale === scale ? result : null;
  const start = Math.max(1, Math.min(page - 4, lastPage - 8));
  const thumbnails = Array.from(
    { length: Math.min(lastPage, 9) },
    (_, index) => start + index,
  );
  if (manifest.totalPages === 0)
    return (
      <div className="flex min-h-0 min-w-0 flex-1 flex-col">
        {header(
          <PreviewActions
            manifest={manifest}
            onReload={onReload}
            notices={manifest.warnings}
            revisionControl={revisionControl}
            warning={manifest.warnings.length > 0}
          />,
        )}
        <p role="status" className="p-4 text-note text-ink-muted">
          {t("assistant.workspace.empty")}
        </p>
      </div>
    );
  const notices = [...manifest.warnings];
  if (initialPage !== undefined && !validInitial)
    notices.push(t("assistant.workspace.modifiedPageUnavailable"));
  if (lastPage < manifest.totalPages)
    notices.push(
      t("assistant.workspace.pageLimit", {
        count: lastPage,
        total: manifest.totalPages,
      }),
    );
  if (visible?.mathFallback)
    notices.push(t("assistant.document.mathCompatibilityPreview"));
  if (visible?.mathUnverified)
    notices.push(t("assistant.workspace.mathUnverified"));
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {header(
        <PreviewActions
          manifest={manifest}
          onReload={onReload}
          notices={notices}
          revisionControl={revisionControl}
          onQuote={() => onQuote(page)}
          quoteDisabled={!visible || failed}
          onPrepare={
            onPrepare ? (action) => onPrepare(page, action) : undefined
          }
          warning={Boolean(
            manifest.warnings.length ||
              visible?.mathUnverified ||
              (initialPage !== undefined && !validInitial) ||
              lastPage < manifest.totalPages,
          )}
        >
          <DropdownMenuSeparator />
          <DropdownMenuLabel>{t("assistant.workspace.zoom")}</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={zoom}
            onValueChange={(value) => {
              if (value === "fit" || value === "100" || value === "200")
                setZoom(value);
            }}
          >
            <DropdownMenuRadioItem value="fit">
              {t("assistant.workspace.fit")}
            </DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="100">100%</DropdownMenuRadioItem>
            <DropdownMenuRadioItem value="200">200%</DropdownMenuRadioItem>
          </DropdownMenuRadioGroup>
          <DropdownMenuCheckboxItem
            checked={showThumbnails}
            onCheckedChange={(checked) => setShowThumbnails(checked === true)}
          >
            {t("assistant.workspace.showThumbnails")}
          </DropdownMenuCheckboxItem>
        </PreviewActions>,
      )}
      <div
        ref={viewport}
        tabIndex={0}
        role="figure"
        aria-label={t("assistant.document.previewPage", { page })}
        aria-busy={!visible && !failed}
        onKeyDown={keys}
        className="relative flex min-h-0 min-w-0 flex-1 items-center justify-center overflow-auto overscroll-contain p-2 outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-brand"
      >
        {visible && !failed ? (
          <img
            key={`${page}:${scale}`}
            src={visible.src}
            alt={t("assistant.document.previewPage", { page })}
            width={visible.width}
            height={visible.height}
            className={
              zoom === "fit"
                ? "h-full min-h-0 w-full object-contain"
                : "max-w-none shrink-0 self-start object-contain"
            }
            style={
              zoom !== "fit"
                ? {
                    width: manifest.width * (zoom === "200" ? 2 : 1),
                    height: "auto",
                  }
                : undefined
            }
            onError={() => {
              if (imageTicketRef.current !== imageTicket) return;
              cache.current.delete(`${page}:${scale}`);
              setFailed(true);
            }}
          />
        ) : !failed ? (
          <PreviewLoading />
        ) : (
          <p role="status" className="p-4 text-note text-ink-muted">
            {t("assistant.workspace.unavailable")}
            {failed && (
              <Button
                variant="secondary"
                className="mt-2"
                onClick={() => {
                  cache.current.delete(`${page}:${scale}`);
                  setAttempt((old) => old + 1);
                }}
              >
                <RotateCcw aria-hidden className="h-3.5 w-3.5" />
                {t("assistant.workspace.retry")}
              </Button>
            )}
          </p>
        )}
      </div>
      {showThumbnails && (
        <nav
          ref={navigation}
          aria-label={t("assistant.workspace.pages")}
          onKeyDown={keys}
          className="flex shrink-0 gap-1 overflow-x-auto border-t border-line px-2 py-1"
        >
          {thumbnails.map((number) => {
            const image =
              cache.current.get(`${number}:1`) ??
              cache.current.get(`${number}:2`);
            return (
              <button
                key={number}
                type="button"
                aria-label={t("assistant.document.previewPage", {
                  page: number,
                })}
                aria-current={number === page ? "page" : undefined}
                className={cn(
                  "flex h-10 w-14 shrink-0 flex-col items-center justify-center overflow-hidden rounded-ctl border border-line bg-card text-note text-ink-muted focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand",
                  number === page && "border-brand",
                )}
                onClick={() => move(number, true)}
              >
                {image ? (
                  <img
                    src={image.src}
                    alt=""
                    className="h-6 w-full object-contain"
                  />
                ) : null}
                <span>{number}</span>
              </button>
            );
          })}
        </nav>
      )}
      {lastPage < manifest.totalPages && (
        <p role="status" className="px-3 py-1 text-note text-warn">
          {t("assistant.workspace.pageLimit", {
            count: lastPage,
            total: manifest.totalPages,
          })}
        </p>
      )}
      <div className="flex min-w-0 shrink-0 items-center gap-1 border-t border-line px-2 py-1">
        <Button
          variant="icon"
          aria-label={t("assistant.document.previousPage")}
          disabled={page === 1}
          onClick={() => move(page - 1)}
        >
          <ChevronLeft aria-hidden className="h-4 w-4" />
        </Button>
        <Popover
          open={jumpOpen}
          onOpenChange={(open) => {
            setJumpOpen(open);
            if (open) setPageInput(String(page));
          }}
        >
          <PopoverTrigger asChild>
            <Button
              variant="ghost"
              size="sm"
              className="mx-auto tabular-nums text-ink-muted"
              aria-label={t("assistant.workspace.pageCounter", {
                page,
                total: manifest.totalPages,
              })}
              title={t("assistant.workspace.goToPage")}
            >
              {t("assistant.workspace.pagePosition", {
                page,
                total: manifest.totalPages,
              })}
            </Button>
          </PopoverTrigger>
          <PopoverContent
            className="w-44 p-2"
            align="center"
            onEscapeKeyDown={(event) => event.stopPropagation()}
            onCloseAutoFocus={(event) => {
              if (jumped.current) {
                event.preventDefault();
                viewport.current?.focus();
                jumped.current = false;
              }
            }}
          >
            <form
              className="flex items-center gap-2"
              onSubmit={(event) => {
                event.preventDefault();
                if (/^\d+$/.test(pageInput)) {
                  jumped.current = true;
                  move(Number(pageInput), true);
                  setJumpOpen(false);
                }
              }}
            >
              <Input
                aria-label={t("assistant.workspace.goToPage")}
                inputMode="numeric"
                value={pageInput}
                onChange={(event) => setPageInput(event.target.value)}
                className="h-8 min-w-0 px-2 text-note"
              />
              <Button type="submit" variant="secondary" size="sm">
                {t("assistant.workspace.go")}
              </Button>
            </form>
          </PopoverContent>
        </Popover>
        <Button
          variant="icon"
          aria-label={t("assistant.document.nextPage")}
          disabled={page === lastPage}
          onClick={() => move(page + 1)}
        >
          <ChevronRight aria-hidden className="h-4 w-4" />
        </Button>
      </div>
    </div>
  );
}

function HtmlPreview({
  request,
  manifest,
  onQuote,
  onReload,
  header,
  revisionControl,
}: {
  header: PreviewHeader;
  revisionControl: ReactNode;
  request: FilePreviewRequest;
  manifest: FilePreviewManifest;
  onQuote: () => void;
  onReload: () => void;
}) {
  const { t } = useTranslation();
  const [result, setResult] = useState<{
    url: string;
    missingResources: string[];
  } | null>(null);
  const [failed, setFailed] = useState(false);
  const [errors, setErrors] = useState<string[]>([]);
  const frame = useRef<HTMLIFrameElement>(null);
  useEffect(() => {
    let current = true;
    setResult(null);
    setFailed(false);
    setErrors([]);
    void assistantPreviewApi
      .html({ ...request, version: manifest.version })
      .then(
        (value) => {
          if (current) setResult(value);
        },
        () => {
          if (current) setFailed(true);
        },
      );
    return () => {
      current = false;
    };
  }, [
    request.sessionId,
    request.expectedOwner,
    request.expectedGeneration,
    request.attachmentId,
    manifest.version,
  ]);
  useEffect(() => {
    if (!result) return;
    const token = new URL(result.url).pathname.split("/")[1];
    const receive = (event: MessageEvent) => {
      const value = event.data;
      if (
        event.source !== frame.current?.contentWindow ||
        !value ||
        typeof value !== "object" ||
        value.type !== "ofox-html-preview-error" ||
        value.token !== token ||
        typeof value.message !== "string" ||
        !value.message.trim() ||
        value.message.length > 500
      )
        return;
      setErrors((old) =>
        old.includes(value.message) || old.length >= 5
          ? old
          : [...old, value.message],
      );
    };
    window.addEventListener("message", receive);
    return () => window.removeEventListener("message", receive);
  }, [result]);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      {header(
        <PreviewActions
          manifest={manifest}
          onReload={onReload}
          notices={[
            t("assistant.workspace.htmlIsolation"),
            ...manifest.warnings,
          ]}
          revisionControl={revisionControl}
          onQuote={onQuote}
          quoteDisabled={!result || failed}
          warning={manifest.warnings.length > 0}
        />,
      )}
      {result?.missingResources.length ? (
        <div
          role="alert"
          className="mx-3 mb-2 max-h-28 shrink-0 overflow-y-auto rounded-ctl border border-line bg-card p-2 text-note text-warn"
        >
          <p>{t("assistant.workspace.missingResources")}</p>
          <ul className="list-inside list-disc">
            {result.missingResources.map((resource, index) => (
              <li key={index} className="break-all">
                {resource}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {errors.length > 0 && (
        <div
          role="alert"
          className="mx-3 mb-2 max-h-28 shrink-0 overflow-y-auto rounded-ctl border border-line bg-card p-2 text-note text-danger"
        >
          <p>{t("assistant.workspace.htmlErrors")}</p>
          <ul className="list-inside list-disc">
            {errors.map((error) => (
              <li key={error} className="break-words">
                {error}
              </li>
            ))}
          </ul>
        </div>
      )}
      {result && !failed ? (
        <iframe
          ref={frame}
          key={result.url}
          src={result.url}
          title={manifest.name}
          sandbox="allow-scripts"
          referrerPolicy="no-referrer"
          className="min-h-0 w-full flex-1 border-0 bg-white"
          onError={() => setFailed(true)}
        />
      ) : failed ? (
        <p role="status" className="p-4 text-note text-ink-muted">
          {t("assistant.workspace.unavailable")}
        </p>
      ) : (
        <PreviewLoading />
      )}
    </div>
  );
}
