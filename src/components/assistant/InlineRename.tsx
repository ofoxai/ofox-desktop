import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Input } from "@/components/ui/input";

/** The longest title the editor accepts (the backend's own limit is wider). */
const TITLE_LIMIT = 120;

/**
 * A conversation's title being edited in place of the heading. Enter saves
 * (and stays open if saving failed), Escape drops the edit, and leaving the
 * field saves it too. A title that is blank or unchanged is never sent.
 * `onClose` tells the page the editor is over, and whether the keyboard
 * ended it, so the page can give focus back.
 */
export function InlineRename({
  title,
  onSave,
  onClose,
}: {
  title: string;
  /** Resolves true once the new title is saved. */
  onSave: (title: string) => Promise<boolean>;
  onClose: (byKeyboard: boolean) => void;
}) {
  const { t } = useTranslation();
  const [value, setValue] = useState(title);
  // The edit is saved, being saved or dropped: the blur that follows (or a
  // second Enter) must not save it again.
  const settled = useRef(false);

  const commit = async (byKeyboard: boolean) => {
    if (settled.current) return;
    const next = value.trim();
    if (!next) {
      // Blank is never saved: Enter keeps editing, leaving drops the edit.
      if (!byKeyboard) {
        settled.current = true;
        onClose(false);
      }
      return;
    }
    settled.current = true;
    if (next === title) {
      onClose(byKeyboard);
      return;
    }
    const saved = await onSave(next);
    if (saved || !byKeyboard) onClose(byKeyboard);
    // Refused after Enter: the title stays open to try again.
    else settled.current = false;
  };

  return (
    <Input
      autoFocus
      aria-label={t("assistant.conversationTitle")}
      className="h-7 min-w-0 max-w-[420px] flex-1 text-ui font-semibold"
      value={value}
      maxLength={TITLE_LIMIT}
      onFocus={(event) => event.currentTarget.select()}
      onChange={(event) => setValue(event.target.value)}
      onBlur={() => void commit(false)}
      onKeyDown={(event) => {
        // The Enter or Escape that ends an IME composition is not ours.
        if (event.nativeEvent.isComposing || event.keyCode === 229) return;
        if (event.key === "Enter") {
          event.preventDefault();
          void commit(true);
        } else if (event.key === "Escape") {
          // Also keeps the app's own Escape from leaving the page.
          event.preventDefault();
          if (settled.current) return;
          settled.current = true;
          onClose(true);
        }
      }}
    />
  );
}
