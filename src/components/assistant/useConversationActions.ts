import {
  useEffect,
  useRef,
  useState,
  type MutableRefObject,
  type RefObject,
} from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import type { AssistantSidebarHandle } from "./AssistantSidebar";
import type { useAssistant } from "./useAssistant";

/** An Undo has to be reachable by keyboard: it stays up longer than a notice. */
const UNDO_TOAST_MS = 8000;
/** Long enough to read the several parts of a result. */
const RESULT_TOAST_MS = 8000;
/** How much of a conversation's title a toast repeats. */
const TOAST_TITLE_CHARS = 40;

const clipTitle = (title: string) => {
  const chars = Array.from(title);
  return chars.length > TOAST_TITLE_CHARS
    ? `${chars.slice(0, TOAST_TITLE_CHARS).join("")}…`
    : title;
};

interface ConversationActionsOptions {
  assistant: ReturnType<typeof useAssistant>;
  sidebarCollapsed: boolean;
  sidebar: RefObject<AssistantSidebarHandle>;
  /** The transcript's scroll view, and whether it follows the end. */
  scroll: RefObject<HTMLDivElement>;
  follow: MutableRefObject<boolean>;
  focusSoon: (target: RefObject<HTMLElement | null>) => void;
  /** The sidebar's entry, or the toggle that opens it while it is collapsed. */
  landOnSidebar: (
    entry: RefObject<HTMLButtonElement>,
  ) => RefObject<HTMLButtonElement>;
  composer: RefObject<HTMLTextAreaElement>;
  sidebarToggle: RefObject<HTMLButtonElement>;
  trashEntry: RefObject<HTMLButtonElement>;
  trashBack: RefObject<HTMLButtonElement>;
  emptyTrashButton: RefObject<HTMLButtonElement>;
}

/**
 * What the page does for conversation management beyond the plain list:
 * moving a conversation to the trash with an Undo, emptying the trash behind a
 * confirmation, and opening a search result at the message it found. It owns
 * the focus each of them leaves behind. Call it after the page's own effects
 * that scroll the transcript: the jump has to come last.
 */
export function useConversationActions({
  assistant,
  sidebarCollapsed,
  sidebar,
  scroll,
  follow,
  focusSoon,
  landOnSidebar,
  composer,
  sidebarToggle,
  trashEntry,
  trashBack,
  emptyTrashButton,
}: ConversationActionsOptions) {
  const { t } = useTranslation();
  // What a toast's Undo needs, as of the render it is pressed in: the toast
  // outlives the render that made it.
  const latest = useRef({ assistant, sidebarCollapsed });
  latest.current = { assistant, sidebarCollapsed };

  /**
   * Move a conversation to the trash and offer to undo it. `onFailure` takes
   * the focus the move did not get to move.
   */
  const moveToTrash = (id: string, title: string, onFailure?: () => void) => {
    const shown = clipTitle(title || t("assistant.untitled"));
    void assistant.setDeleted(true, id).then((saved) => {
      if (!saved) {
        onFailure?.();
        return;
      }
      focusSoon(landOnSidebar(trashEntry));
      toast(t("assistant.trashed", { title: shown }), {
        duration: UNDO_TOAST_MS,
        action: { label: t("assistant.undo"), onClick: () => undoTrash(id) },
      });
    });
  };

  // Restore the conversation the toast is about, even if the selection has
  // moved on since.
  const undoTrash = (id: string) =>
    void latest.current.assistant.setDeleted(false, id).then((restored) => {
      if (!restored) return;
      const { assistant: now, sidebarCollapsed: collapsed } = latest.current;
      // Nothing is open: but that is also the composer of a new conversation,
      // and opening one would replace what is written in it.
      const unsent = now.draft !== "" || now.attachments.length > 0;
      if (!now.selectedId && !now.showTrash && !unsent) {
        // Back to the conversation it was.
        now.select(id);
        focusSoon(composer);
      } else if (collapsed) focusSoon(sidebarToggle);
      else if (now.showTrash) focusSoon(trashBack);
      else sidebar.current?.focusRow(id);
    });

  // Emptying the trash is being confirmed: how many conversations the dialog
  // says it holds.
  const [confirmEmpty, setConfirmEmpty] = useState<number | null>(null);
  // The trash was emptied: the dialog's focus goes to the way back, not to
  // the button that has just run out of work.
  const trashEmptied = useRef(false);
  useEffect(() => setConfirmEmpty(null), [assistant.showTrash]);
  const emptyTrash = {
    open: confirmEmpty !== null && assistant.showTrash,
    count: confirmEmpty ?? 0,
    ask: () => {
      assistant.setError(null);
      trashEmptied.current = false;
      setConfirmEmpty(assistant.trashCount);
    },
    cancel: () => {
      setConfirmEmpty(null);
      assistant.setError(null);
    },
    confirm: () =>
      void assistant.emptyTrash().then((report) => {
        // Not done: the dialog stays, with the reason.
        if (!report) return;
        trashEmptied.current = true;
        setConfirmEmpty(null);
        if (report.failed > 0)
          toast.warning(
            t("assistant.emptyTrashPartial", {
              deleted: report.deleted,
              failed: report.failed,
            }),
            { duration: RESULT_TOAST_MS },
          );
        else if (report.deleted > 0)
          toast.success(
            t("assistant.emptyTrashDone", { count: report.deleted }),
          );
        else toast(t("assistant.trashEmpty"));
      }),
    onCloseAutoFocus: (event: Event) => {
      event.preventDefault();
      (trashEmptied.current
        ? landOnSidebar(trashBack)
        : emptyTrashButton
      ).current?.focus();
    },
  };

  // The message a search found in the conversation being opened, until the
  // transcript has scrolled to it.
  const [jump, setJump] = useState<{
    sessionId: string;
    messageId: string;
  } | null>(null);
  const { selectedId, loading, detail } = assistant;
  useEffect(() => {
    if (!jump) return;
    // Another conversation is open, or this one is still on its way.
    if (selectedId !== jump.sessionId) {
      setJump(null);
      return;
    }
    if (loading) return;
    // Done either way: a conversation that failed to load must not scroll
    // when some later load brings it in.
    setJump(null);
    if (detail?.session.id !== jump.sessionId) return;
    const target = [
      ...(scroll.current?.querySelectorAll("[data-message-id]") ?? []),
    ].find(
      (element) => element.getAttribute("data-message-id") === jump.messageId,
    );
    if (!target) return;
    // Not the end: new content below must not pull the view away again.
    follow.current = false;
    target.scrollIntoView?.({ block: "center" });
  }, [jump, selectedId, loading, detail, scroll, follow]);

  /** A row chosen in the list; for a search result, also the message to show. */
  const openFromList = (id: string, messageId?: string) => {
    setJump(messageId ? { sessionId: id, messageId } : null);
    assistant.select(id);
  };

  return { moveToTrash, openFromList, emptyTrash };
}
