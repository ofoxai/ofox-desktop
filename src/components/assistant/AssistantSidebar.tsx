import {
  forwardRef,
  useEffect,
  useId,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import {
  AlertTriangle,
  ArrowLeft,
  Download,
  Ellipsis,
  Loader2,
  Pin,
  PinOff,
  Search,
  SquarePen,
  Trash2,
  Upload,
} from "lucide-react";
import { assistantErrorKey, type AssistantSession } from "@/lib/api/assistant";
import { isMac } from "@/lib/platform";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { highlightText } from "@/components/sessions/utils";
import { searchHighlight } from "@/utils/assistantSearch";
import { groupSessions } from "@/utils/assistantSessionGroups";
import { cn } from "@/lib/utils";
import type { AssistantTransferResult, useAssistant } from "./useAssistant";
import { usePendingFocus } from "./usePendingFocus";
import { useStartOfToday } from "./useStartOfToday";

/** Long enough to read a restore's several lines. */
const TRANSFER_TOAST_MS = 8000;

/**
 * What a finished backup or restore did: one line for each thing that
 * happened, and `warn` for those that left something undone.
 */
function transferLines(
  result: AssistantTransferResult,
  t: TFunction,
): { text: string; warn?: boolean }[] {
  if (result.kind === "backup")
    return [
      { text: t("assistant.backupSaved", { count: result.sessions }) },
      ...(result.skipped > 0
        ? [
            {
              text: t("assistant.backupSkipped", { count: result.skipped }),
              warn: true,
            },
          ]
        : []),
    ];
  const lines = [
    result.imported > 0 && {
      text: t("assistant.restoreImported", { count: result.imported }),
    },
    result.skipped > 0 && {
      text: t("assistant.restoreSkipped", { count: result.skipped }),
    },
    result.conflicts > 0 && {
      text: t("assistant.restoreConflicts", { count: result.conflicts }),
      warn: true,
    },
  ].filter((line) => line !== false);
  return lines.length
    ? lines
    : [{ text: t("assistant.restoreNothing"), warn: true }];
}

/** Tell each new backup or restore result once, as a toast. */
function useTransferToast(result: AssistantTransferResult | null) {
  const { t } = useTranslation();
  const told = useRef(result);
  useEffect(() => {
    if (!result || result === told.current) return;
    told.current = result;
    const [first, ...rest] = transferLines(result, t);
    const notify = [first, ...rest].some((line) => line.warn)
      ? toast.warning
      : toast.success;
    notify(first.text, {
      description: rest.length
        ? rest.map((line) => line.text).join("\n")
        : undefined,
      duration: TRANSFER_TOAST_MS,
      classNames: { description: "whitespace-pre-line" },
    });
  }, [result, t]);
}

interface AssistantSidebarProps {
  id: string;
  collapsed: boolean;
  assistant: ReturnType<typeof useAssistant>;
  /** A new conversation can be started now. */
  canCreate: boolean;
  /** Start one: the same action as the keyboard shortcut. */
  onCreate: () => void;
  /**
   * A row was chosen. `messageId` is the message a search found in it, which
   * the page scrolls to; none for a conversation found by its title.
   */
  onOpen: (id: string, messageId?: string) => void;
  /** Move a conversation to the trash: the page tells it and offers Undo. */
  onMoveToTrash: (item: AssistantSession) => void;
  /** Ask to empty the trash (the page confirms it). */
  onEmptyTrash: () => void;
  newConversationRef: RefObject<HTMLButtonElement>;
  trashEntryRef: RefObject<HTMLButtonElement>;
  trashBackRef: RefObject<HTMLButtonElement>;
  emptyTrashRef: RefObject<HTMLButtonElement>;
  /** Back up or restore; `trigger` takes focus back once it is over. */
  onTransfer: (
    kind: "backup" | "restore",
    trigger: HTMLButtonElement | null,
  ) => void;
}

export interface AssistantSidebarHandle {
  /**
   * Give focus to a conversation's row once it is listed: where a pin or an
   * undo puts it, which is only known after the list is read again. With
   * `pinned`, once it is listed as pinned (or not).
   */
  focusRow: (id: string, pinned?: boolean) => void;
}

/** How long a requested focus waits for its row to be listed. */
const ROW_FOCUS_WAIT_MS = 3000;

/**
 * The conversation list, in groups by date with the pinned ones first:
 * search and new conversation on top, the trash and backup/restore in the
 * footer. In the trash it lists trashed conversations, ungrouped, under a way
 * back and the means to empty it.
 */
export const AssistantSidebar = forwardRef<
  AssistantSidebarHandle,
  AssistantSidebarProps
>(function AssistantSidebar(
  {
    id,
    collapsed,
    assistant,
    canCreate,
    onCreate,
    onOpen,
    onMoveToTrash,
    onEmptyTrash,
    newConversationRef,
    trashEntryRef,
    trashBackRef,
    emptyTrashRef,
    onTransfer,
  },
  handle,
) {
  const { t, i18n } = useTranslation();
  const [searchOpen, setSearchOpen] = useState(false);
  const searchButton = useRef<HTMLButtonElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const backupHint = useId();
  const rowIds = useId();
  const focusSoon = usePendingFocus();
  const rowButtons = useRef(new Map<string, HTMLButtonElement>());
  const wantedRow = useRef<{
    id: string;
    pinned?: boolean;
    until: number;
  } | null>(null);
  const today = useStartOfToday();
  const { showTrash, transferring, trashCount } = assistant;
  const highlight = searchHighlight(assistant.search);
  // A search with text stays open: its filter is in force.
  const searchShown = searchOpen || assistant.search !== "";
  const transferLocked =
    assistant.busy || assistant.loading || !assistant.bootstrap?.owner;
  useTransferToast(assistant.transferResult);
  const groups = useMemo(
    () => (showTrash ? [] : groupSessions(assistant.sessions, today)),
    [assistant.sessions, showTrash, today],
  );

  // The wanted row takes focus once it is listed (where it is to be). A row
  // that never is, say a filter hides it, stops waiting.
  const focusWantedRow = () => {
    const wanted = wantedRow.current;
    if (!wanted) return;
    if (Date.now() > wanted.until) {
      wantedRow.current = null;
      return;
    }
    const item = assistant.sessions.find((listed) => listed.id === wanted.id);
    const button = rowButtons.current.get(wanted.id);
    if (!item || !button) return;
    if (
      wanted.pinned !== undefined &&
      (item.pinnedAt !== null) !== wanted.pinned
    )
      return;
    wantedRow.current = null;
    button.focus();
  };
  useEffect(focusWantedRow);
  const wantRow = (rowId: string, pinned?: boolean) => {
    wantedRow.current = {
      id: rowId,
      pinned,
      until: Date.now() + ROW_FOCUS_WAIT_MS,
    };
    focusWantedRow();
  };
  useImperativeHandle(handle, () => ({ focusRow: wantRow }));
  const togglePin = (item: AssistantSession) => {
    const pinned = item.pinnedAt === null;
    wantRow(item.id, pinned);
    void assistant.setPinned(item.id, pinned).then((saved) => {
      if (!saved) wantedRow.current = null;
    });
  };

  // Every row is dated on each render, and the list renders with each
  // streamed frame: one formatter per language, not one per row.
  const stampFormat = useMemo(
    () =>
      new Intl.DateTimeFormat(i18n.language, {
        dateStyle: "medium",
        timeStyle: "short",
      }),
    [i18n.language],
  );
  const stamp = (at: number) => stampFormat.format(at);

  const renderRow = (item: AssistantSession) => {
    const selected = assistant.selectedId === item.id;
    const title = item.title || t("assistant.untitled");
    const rowId = `${rowIds}-${item.id}`;
    return (
      <li key={item.id} className="group relative">
        <button
          ref={(element) => {
            if (element) rowButtons.current.set(item.id, element);
            else rowButtons.current.delete(item.id);
          }}
          id={rowId}
          type="button"
          disabled={transferring}
          // The title may be cut short in the row: here it is whole.
          title={`${title}\n${stamp(item.updatedAt)}`}
          aria-current={selected ? "page" : undefined}
          onClick={() => onOpen(item.id, assistant.matches[item.id])}
          className={cn(
            // The room the menu button takes while it shows: the title (and
            // the spinner) end before it, not under it.
            "flex w-full items-center gap-1.5 rounded-ctl px-2 py-1.5 text-left text-ctl transition-colors hover:bg-pill group-hover:pr-8 group-focus-within:pr-8 group-has-[[data-state=open]]:pr-8 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand disabled:cursor-wait disabled:opacity-50",
            selected && "bg-brand-soft text-brand-ink",
          )}
        >
          <span className="min-w-0 flex-1 truncate">
            {highlightText(title, highlight)}
          </span>
          {item.activeRun && (
            <>
              <Loader2
                aria-hidden
                className="h-3.5 w-3.5 shrink-0 animate-spin text-ink-muted motion-reduce:animate-none"
              />
              <span className="sr-only">{t("assistant.running")}</span>
            </>
          )}
        </button>
        {!showTrash && (
          <DropdownMenu modal={false}>
            <DropdownMenuTrigger asChild>
              {/* Over the row's end, shown while the pointer or focus is in
                  the row; it stays in the tab order. */}
              <Button
                variant="icon"
                aria-label={t("assistant.rowActions")}
                aria-describedby={rowId}
                disabled={transferring}
                className="absolute right-1 top-1/2 h-6 w-6 -translate-y-1/2 opacity-0 focus-visible:opacity-100 group-focus-within:opacity-100 group-hover:opacity-100 data-[state=open]:bg-pill data-[state=open]:text-ink data-[state=open]:opacity-100"
              >
                <Ellipsis aria-hidden className="h-4 w-4" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent
              align="end"
              className="min-w-[140px] max-w-[280px]"
            >
              <DropdownMenuItem onSelect={() => togglePin(item)}>
                {item.pinnedAt === null ? (
                  <Pin aria-hidden className="h-3.5 w-3.5 shrink-0" />
                ) : (
                  <PinOff aria-hidden className="h-3.5 w-3.5 shrink-0" />
                )}
                {t(
                  item.pinnedAt === null ? "assistant.pin" : "assistant.unpin",
                )}
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem
                disabled={assistant.busy}
                className="text-danger"
                onSelect={() => onMoveToTrash(item)}
              >
                <Trash2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
                {t("assistant.moveToTrash")}
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        )}
      </li>
    );
  };
  const unreadableRows = assistant.unreadable.map((item) => (
    <li
      key={item.id}
      className="rounded-ctl px-2 py-1.5 text-ctl text-ink-muted"
    >
      <span className="flex items-center gap-1.5 font-medium text-danger">
        <AlertTriangle aria-hidden className="h-3.5 w-3.5" />
        {t("assistant.unreadable")}
      </span>
      <span className="mt-0.5 block text-note">
        {t(assistantErrorKey(item.error))}
      </span>
    </li>
  ));

  return (
    <aside
      id={id}
      hidden={collapsed}
      // `flex` would win over the hidden attribute's display: none.
      className={cn(
        "w-[200px] shrink-0 flex-col gap-2 border-r border-line bg-card-2 p-3 xl:w-[240px]",
        collapsed ? "hidden" : "flex",
      )}
      aria-label={t("assistant.conversations")}
    >
      <div className="flex h-[26px] shrink-0 items-center gap-1">
        {searchShown ? (
          <SearchInput
            autoFocus
            className="w-auto min-w-0 flex-1"
            aria-label={t("assistant.search")}
            placeholder={t("assistant.search")}
            value={assistant.search}
            onChange={assistant.setSearch}
            onKeyDown={(event) => {
              if (event.key !== "Escape") return;
              // The Escape that cancels an IME candidate is not ours.
              if (event.nativeEvent.isComposing || event.keyCode === 229)
                return;
              // Also keeps the app's own Escape from leaving the page.
              event.preventDefault();
              assistant.setSearch("");
              setSearchOpen(false);
              focusSoon(searchButton);
            }}
            onBlur={(event) => {
              if (!event.currentTarget.value) setSearchOpen(false);
            }}
          />
        ) : (
          <>
            <Button
              ref={searchButton}
              variant="icon"
              aria-label={t("assistant.search")}
              title={t("assistant.search")}
              onClick={() => setSearchOpen(true)}
            >
              <Search aria-hidden className="h-3.5 w-3.5" />
            </Button>
            <span className="flex-1" />
          </>
        )}
        <Button
          ref={newConversationRef}
          variant="icon"
          aria-label={t("assistant.newConversation")}
          aria-keyshortcuts={isMac() ? "Meta+N" : "Control+N"}
          title={t("assistant.newConversationShortcut", {
            shortcut: isMac() ? "⌘N" : "Ctrl+N",
          })}
          disabled={!canCreate}
          onClick={onCreate}
        >
          <SquarePen aria-hidden className="h-3.5 w-3.5" />
        </Button>
      </div>
      {showTrash && (
        <div className="flex shrink-0 items-center gap-1">
          <Button
            ref={trashBackRef}
            variant="icon"
            aria-label={t("assistant.backToConversations")}
            title={t("assistant.backToConversations")}
            disabled={transferring}
            onClick={() => {
              assistant.viewTrash(false);
              focusSoon(trashEntryRef);
            }}
          >
            <ArrowLeft aria-hidden className="h-3.5 w-3.5" />
          </Button>
          <h2 className="min-w-0 flex-1 truncate text-ctl font-semibold text-ink">
            {t("assistant.trash")}
          </h2>
          <Button
            ref={emptyTrashRef}
            variant="dangerGhost"
            className="shrink-0 px-2"
            aria-label={t("assistant.emptyTrashAction")}
            disabled={
              trashCount === 0 ||
              transferring ||
              assistant.busy ||
              assistant.loading
            }
            onClick={onEmptyTrash}
          >
            {t("assistant.emptyTrash")}
          </Button>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        {showTrash ? (
          <ul className="space-y-0.5" aria-label={t("assistant.trash")}>
            {assistant.sessions.map(renderRow)}
            {unreadableRows}
          </ul>
        ) : (
          <ul aria-label={t("assistant.conversations")}>
            {groups.map((group) => (
              <li key={group.id}>
                <h2
                  id={`${rowIds}-group-${group.id}`}
                  className="px-2 pb-0.5 pt-2 text-note font-medium text-ink-ctl"
                >
                  {t(`assistant.groups.${group.id}`)}
                </h2>
                <ul
                  className="space-y-0.5"
                  aria-labelledby={`${rowIds}-group-${group.id}`}
                >
                  {group.sessions.map(renderRow)}
                </ul>
              </li>
            ))}
            {unreadableRows}
          </ul>
        )}
        {!assistant.sessions.length && !assistant.unreadable.length && (
          <p className="px-2 py-6 text-center text-note text-ink-muted">
            {t(
              assistant.search
                ? "assistant.noMatches"
                : showTrash
                  ? "assistant.trashEmpty"
                  : "assistant.noConversations",
            )}
          </p>
        )}
      </div>
      <div className="flex shrink-0 items-center gap-1 border-t border-line pt-2">
        {!showTrash && (
          <Button
            ref={trashEntryRef}
            variant="ghost"
            className="min-w-0 flex-1 justify-start px-2"
            disabled={transferring}
            onClick={() => {
              assistant.viewTrash(true);
              focusSoon(trashBackRef);
            }}
          >
            <Trash2 aria-hidden className="h-3.5 w-3.5 shrink-0" />
            <span className="truncate">
              {trashCount > 0
                ? t("assistant.trashWithCount", { count: trashCount })
                : t("assistant.trash")}
            </span>
          </Button>
        )}
        <DropdownMenu modal={false}>
          <DropdownMenuTrigger asChild>
            <Button
              ref={menuTrigger}
              variant="icon"
              aria-label={t("assistant.moreActions")}
              title={t("assistant.moreActions")}
              className="ml-auto shrink-0 data-[state=open]:bg-pill data-[state=open]:text-ink"
            >
              <Ellipsis aria-hidden className="h-4 w-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent
            align="end"
            side="top"
            className="min-w-[160px] max-w-[280px]"
          >
            <DropdownMenuItem
              disabled={transferLocked}
              title={t("assistant.backupHint")}
              aria-describedby={backupHint}
              onSelect={() => onTransfer("backup", menuTrigger.current)}
            >
              <Download aria-hidden className="h-3.5 w-3.5 shrink-0" />
              {t("assistant.backupMenu")}
            </DropdownMenuItem>
            <DropdownMenuItem
              disabled={transferLocked}
              title={t("assistant.backupHint")}
              aria-describedby={backupHint}
              onSelect={() => onTransfer("restore", menuTrigger.current)}
            >
              <Upload aria-hidden className="h-3.5 w-3.5 shrink-0" />
              {t("assistant.restoreMenu")}
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
        <span id={backupHint} hidden>
          {t("assistant.backupHint")}
        </span>
      </div>
    </aside>
  );
});
