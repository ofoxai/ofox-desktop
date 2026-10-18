import {
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { Resource } from "./LibrarySection";
import { DraftStore } from "./useDraftState";

export interface LibraryEditorController {
  dirty: boolean;
  saving: boolean;
  save: () => Promise<boolean>;
  discard: () => void;
}
export type LibraryLeaveGuard = (proceed: () => void) => void;

export function libraryScroller(
  element: HTMLElement | null,
): HTMLElement | undefined {
  let parent = element?.parentElement;
  while (parent) {
    if (/(auto|scroll)/.test(getComputedStyle(parent).overflowY)) return parent;
    parent = parent.parentElement;
  }
}

/** Editing and navigation are separate from the page's mutation/dialog coordinator. */
export function useLibraryInline(
  rows: Resource[],
  epoch: number | undefined,
  routingKey: string,
  active: boolean,
) {
  const [opened, setOpened] = useState<{
    invalidated?: boolean;
    row: Resource;
    epoch: number;
    routingKey: string;
    store: DraftStore;
  } | null>(null);
  const [dirty, setDirty] = useState(false);
  const [asking, setAsking] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);
  const stores = useRef(new Map<string, DraftStore>());
  useEffect(() => {
    stores.current.clear();
    setOpened((previous) =>
      previous ? { ...previous, invalidated: true } : previous,
    );
  }, [epoch, routingKey]);
  const editor = useRef<LibraryEditorController>();
  const pending = useRef<() => void>();
  const refreshedFocus = useRef<{
    id: string;
    selector: string;
    index: number;
    text?: string;
  }>();
  function storeFor(row: Resource) {
    const key = `${epoch}:${routingKey}:${row.id}:${row.revision}`;
    let store = stores.current.get(key);
    if (!store) {
      store = new DraftStore();
      stores.current.set(key, store);
      if (stores.current.size > 32)
        stores.current.delete(stores.current.keys().next().value!);
    }
    return store;
  }
  const anchor = useRef<{
    element: HTMLElement;
    top: number;
    focus: boolean;
  }>();
  const guard = useCallback<LibraryLeaveGuard>((proceed) => {
    if (!editor.current?.dirty) {
      proceed();
      return;
    }
    if (pending.current) return;
    pending.current = proceed;
    setAsking(true);
  }, []);
  const registerEditor = useCallback((controller: LibraryEditorController) => {
    editor.current = controller;
    setDirty(controller.dirty);
  }, []);
  const toggle = (row: Resource, trigger: HTMLElement) =>
    guard(() => {
      anchor.current = {
        element: trigger,
        top: trigger.getBoundingClientRect().top,
        focus: true,
      };
      editor.current = undefined;
      setDirty(false);
      const store = storeFor(row);
      setOpened((previous) =>
        previous?.row.id === row.id
          ? null
          : { row, epoch: epoch ?? -1, routingKey, store },
      );
    });
  useLayoutEffect(() => {
    const restore = refreshedFocus.current;
    if (restore && restore.id === opened?.row.id) {
      refreshedFocus.current = undefined;
      const nodes = Array.from(
        document
          .getElementById(`library-inline-${restore.id}`)
          ?.querySelectorAll<HTMLElement>(restore.selector) ?? [],
      );
      const target = restore.text
        ? nodes.find((node) => node.textContent === restore.text)
        : nodes[restore.index];
      target?.focus({ preventScroll: true });
    }
    const saved = anchor.current;
    if (!saved?.element.isConnected) return;
    anchor.current = undefined;
    const scroll = libraryScroller(saved.element);
    if (scroll)
      scroll.scrollTop += saved.element.getBoundingClientRect().top - saved.top;
    if (saved.focus) saved.element.focus({ preventScroll: true });
  }, [opened]);
  const current = opened
    ? rows.find((row) => row.id === opened.row.id)
    : undefined;
  const stale =
    !!opened &&
    (opened.invalidated ||
      !current ||
      current.revision !== opened.row.revision ||
      epoch !== opened.epoch ||
      routingKey !== opened.routingKey);
  useEffect(() => {
    if (!stale || dirty || !opened) return;
    editor.current = undefined;
    const region = document.getElementById(`library-inline-${opened.row.id}`),
      focused = document.activeElement;
    if (
      active &&
      current &&
      opened.epoch === epoch &&
      opened.routingKey === routingKey &&
      focused instanceof HTMLElement &&
      region?.contains(focused)
    ) {
      const selector = focused.isContentEditable
        ? '[contenteditable="true"]'
        : focused.tagName.toLowerCase();
      const nodes = Array.from(region.querySelectorAll(selector));
      refreshedFocus.current = {
        id: current.id,
        selector,
        index: nodes.indexOf(focused),
        ...(focused.tagName === "BUTTON"
          ? { text: focused.textContent ?? "" }
          : {}),
      };
    }
    setOpened(
      current
        ? {
            row: current,
            epoch: epoch ?? -1,
            routingKey,
            store: storeFor(current),
          }
        : null,
    );
  }, [stale, dirty, current, epoch, routingKey, opened]);
  useEffect(() => {
    if (!dirty || !active) return;
    const before = (event: BeforeUnloadEvent) => {
      event.preventDefault();
      event.returnValue = "";
    };
    window.addEventListener("beforeunload", before);
    // Native close hides the retained main window without discarding this editor.
    // Only a real document unload needs the browser's unsaved-content prompt.
    return () => window.removeEventListener("beforeunload", before);
  }, [dirty, active, guard]);
  async function resolve(save: boolean) {
    if (saving) return;
    if (save) {
      if (stale || !editor.current) return;
      setSaving(true);
      let succeeded = false;
      try {
        succeeded = await editor.current.save();
      } finally {
        setSaving(false);
      }
      if (!succeeded) {
        setSaveError(true);
        return;
      }
    } else editor.current?.discard();
    const next = pending.current;
    pending.current = undefined;
    setAsking(false);
    setSaveError(false);
    setDirty(false);
    next?.();
  }
  function cancel() {
    pending.current = undefined;
    setAsking(false);
    setSaveError(false);
  }
  const resetEditor = useCallback(() => {
    editor.current = undefined;
    setDirty(false);
  }, []);
  function returnFocus() {
    const container =
      document.querySelector<HTMLElement>("[data-library-editor-dialog]") ||
      (opened && document.getElementById(`library-inline-${opened.row.id}`));
    (container?.querySelector("textarea,input") as HTMLElement | null)?.focus({
      preventScroll: true,
    });
  }
  return {
    opened,
    dirty,
    stale,
    toggle,
    guard,
    registerEditor,
    resetEditor,
    asking,
    saving,
    saveError,
    resolve,
    cancel,
    returnFocus,
  };
}

export function LibraryUnsavedDialog({
  state,
}: {
  state: ReturnType<typeof useLibraryInline>;
}) {
  const { t } = useTranslation();
  return (
    <Dialog
      open={state.asking}
      onOpenChange={(open) => {
        if (!open && !state.saving) state.cancel();
      }}
    >
      <DialogContent
        zIndex="alert"
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          state.returnFocus();
        }}
        onEscapeKeyDown={(event) => {
          if (state.saving) event.preventDefault();
        }}
      >
        <DialogHeader>
          <DialogTitle>{t("library.interaction.unsavedTitle")}</DialogTitle>
          <DialogDescription>
            {t("library.interaction.unsavedDescription")}
          </DialogDescription>
        </DialogHeader>
        {state.saveError && (
          <p role="alert" className="px-4 text-ctl text-danger">
            {t("library.failed")}
          </p>
        )}
        <DialogFooter>
          <Button
            variant="ghost"
            disabled={state.saving}
            onClick={state.cancel}
          >
            {t("library.interaction.keepEditing")}
          </Button>
          <Button
            variant="secondary"
            disabled={state.saving}
            onClick={() => void state.resolve(false)}
          >
            {t("library.interaction.discardContinue")}
          </Button>
          <Button
            disabled={state.saving || state.stale}
            onClick={() => void state.resolve(true)}
          >
            {t("library.interaction.saveContinue")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
