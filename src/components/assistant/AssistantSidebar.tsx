import { useEffect, useId, useRef, useState, type RefObject } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
import { toast } from "sonner";
import {
  AlertTriangle,
  ArrowLeft,
  Download,
  Ellipsis,
  Loader2,
  Search,
  SquarePen,
  Trash2,
  Upload,
} from "lucide-react";
import { assistantErrorKey } from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import { SearchInput } from "@/components/ui/search-input";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/lib/utils";
import type { AssistantTransferResult, useAssistant } from "./useAssistant";
import { usePendingFocus } from "./usePendingFocus";

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
  newConversationRef: RefObject<HTMLButtonElement>;
  trashEntryRef: RefObject<HTMLButtonElement>;
  trashBackRef: RefObject<HTMLButtonElement>;
  /** Back up or restore; `trigger` takes focus back once it is over. */
  onTransfer: (
    kind: "backup" | "restore",
    trigger: HTMLButtonElement | null,
  ) => void;
}

/**
 * The conversation list: search and new conversation on top, the trash and
 * backup/restore in the footer. In the trash it lists trashed conversations
 * under a way back.
 */
export function AssistantSidebar({
  id,
  collapsed,
  assistant,
  canCreate,
  newConversationRef,
  trashEntryRef,
  trashBackRef,
  onTransfer,
}: AssistantSidebarProps) {
  const { t, i18n } = useTranslation();
  const [searchOpen, setSearchOpen] = useState(false);
  const searchButton = useRef<HTMLButtonElement>(null);
  const menuTrigger = useRef<HTMLButtonElement>(null);
  const backupHint = useId();
  const focusSoon = usePendingFocus();
  const { showTrash, transferring, trashCount } = assistant;
  // A search with text stays open: its filter is in force.
  const searchShown = searchOpen || assistant.search !== "";
  const transferLocked =
    assistant.busy || assistant.loading || !assistant.bootstrap?.owner;
  useTransferToast(assistant.transferResult);

  const stamp = (at: number) =>
    new Date(at).toLocaleString(i18n.language, {
      dateStyle: "medium",
      timeStyle: "short",
    });

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
          title={t("assistant.newConversation")}
          disabled={!canCreate}
          onClick={() => void assistant.create()}
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
          <h2 className="min-w-0 truncate text-ctl font-semibold text-ink">
            {t("assistant.trash")}
          </h2>
        </div>
      )}
      <div className="min-h-0 flex-1 overflow-y-auto">
        <ul
          className="space-y-0.5"
          aria-label={t(
            showTrash ? "assistant.trash" : "assistant.conversations",
          )}
        >
          {assistant.sessions.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                disabled={transferring}
                title={stamp(item.updatedAt)}
                aria-current={
                  assistant.selectedId === item.id ? "page" : undefined
                }
                onClick={() => assistant.select(item.id)}
                className={cn(
                  "flex w-full items-center gap-1.5 rounded-ctl px-2 py-1.5 text-left text-ctl transition-colors hover:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand disabled:cursor-wait disabled:opacity-50",
                  assistant.selectedId === item.id &&
                    "bg-brand-soft text-brand-ink",
                )}
              >
                <span className="min-w-0 flex-1 truncate">
                  {item.title || t("assistant.untitled")}
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
            </li>
          ))}
          {assistant.unreadable.map((item) => (
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
          ))}
        </ul>
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
}
