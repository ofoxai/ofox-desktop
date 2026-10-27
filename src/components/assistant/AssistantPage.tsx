import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertTriangle,
  ArrowUp,
  ArrowUpRight,
  Loader2,
  MessageSquare,
  Pencil,
  Plus,
  Square,
  X,
} from "lucide-react";
import { assistantErrorKey } from "@/lib/api/assistant";
import { useAssistant } from "./useAssistant";
import { AssistantMessageView } from "./AssistantMessageView";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { MenuSelect } from "@/components/ui/menu-select";
import { SearchInput } from "@/components/ui/search-input";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useShellRefresh } from "@/components/shell/ShellContext";
import { cn } from "@/lib/utils";

const STARTERS = ["summary", "writing", "explain"] as const;

interface AssistantPageProps {
  enabled?: boolean;
  onLoginRequested?: () => void;
}

/** Signed out: one way in, and nothing loaded or registered. */
export function AssistantLoginRequired({
  onLoginRequested,
}: Pick<AssistantPageProps, "onLoginRequested">) {
  const { t } = useTranslation();
  return (
    <section className="grid h-full min-h-0 place-items-center rounded-list border border-line bg-card px-6 py-10 text-center">
      <div className="flex max-w-sm flex-col items-center">
        <div className="mb-5 grid h-11 w-11 place-items-center rounded-card bg-brand-soft text-brand-ink">
          <MessageSquare aria-hidden className="h-5 w-5" />
        </div>
        <h1 className="text-title font-semibold text-ink">
          {t("assistant.title")}
        </h1>
        <p className="mb-6 mt-2 text-ui leading-relaxed text-ink-muted">
          {t("assistant.loginRequired")}
        </p>
        <Button onClick={onLoginRequested}>{t("toolCatalog.login")}</Button>
      </div>
    </section>
  );
}

export function AssistantPage({
  enabled = true,
  onLoginRequested,
}: AssistantPageProps) {
  return enabled ? (
    <AssistantWorkspace />
  ) : (
    <AssistantLoginRequired onLoginRequested={onLoginRequested} />
  );
}

function AssistantWorkspace() {
  const { t, i18n } = useTranslation();
  const assistant = useAssistant(true);
  const { detail, bootstrap, run, busy } = assistant;
  const [rename, setRename] = useState<string | null>(null);
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const composer = useRef<HTMLTextAreaElement>(null);
  const focusStarter = useRef(false);
  const session = detail?.session;
  const available = Boolean(
    bootstrap?.runtime.available &&
      bootstrap.models.length > 0 &&
      assistant.connected,
  );
  const writable = available && !run && !busy && !assistant.loading;
  // Tool, system and other internal records stay out of the transcript.
  const messages = (detail?.messages ?? []).filter(
    (message) => message.role === "user" || message.role === "assistant",
  );

  useShellRefresh("assistant", async () => {
    await assistant.loadBootstrap();
    await assistant.refreshSessions();
    if (assistant.selectedId)
      await assistant.refreshDetail(assistant.selectedId);
  });

  useEffect(() => {
    setRename(null);
    follow.current = true;
  }, [assistant.selectedId]);
  useEffect(() => {
    if (follow.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [assistant.stream, messages.length]);
  useLayoutEffect(() => {
    if (!focusStarter.current) return;
    focusStarter.current = false;
    const input = composer.current;
    if (input && !input.disabled) {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }
  }, [assistant.draft]);

  const send = () => {
    follow.current = true;
    void assistant.send().then((sent) => {
      if (sent) composer.current?.focus();
    });
  };
  const stop = () =>
    void assistant.stop().then(() => composer.current?.focus());
  const saveTitle = () => {
    const title = rename?.trim();
    if (title)
      void assistant.update({ title }).then((saved) => {
        if (saved) setRename(null);
      });
  };

  return (
    <div className="flex h-full min-h-0 overflow-hidden rounded-list border border-line bg-card">
      <aside
        className="flex w-52 shrink-0 flex-col gap-2 border-r border-line bg-card-2 p-3"
        aria-label={t("assistant.conversations")}
      >
        <Button
          className="w-full"
          disabled={!available || busy}
          onClick={() => void assistant.create()}
        >
          <Plus aria-hidden className="h-3.5 w-3.5" />
          {t("assistant.newConversation")}
        </Button>
        <SearchInput
          className="w-full"
          aria-label={t("assistant.search")}
          placeholder={t("assistant.search")}
          value={assistant.search}
          onChange={assistant.setSearch}
        />
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ul className="space-y-1" aria-label={t("assistant.conversations")}>
            {assistant.sessions.map((item) => (
              <li key={item.id}>
                <button
                  type="button"
                  aria-current={
                    assistant.selectedId === item.id ? "page" : undefined
                  }
                  onClick={() => assistant.select(item.id)}
                  className={cn(
                    "w-full rounded-ctl px-2 py-2 text-left text-ctl transition-colors hover:bg-pill focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-brand",
                    assistant.selectedId === item.id &&
                      "bg-brand-soft text-brand-ink",
                  )}
                >
                  <span className="block truncate">
                    {item.title || t("assistant.untitled")}
                  </span>
                  <span className="mt-0.5 block truncate text-note text-ink-muted">
                    {item.activeRun
                      ? t("assistant.running")
                      : new Date(item.updatedAt).toLocaleDateString(
                          i18n.language,
                        )}
                  </span>
                </button>
              </li>
            ))}
            {assistant.unreadable.map((item) => (
              <li
                key={item.id}
                className="rounded-ctl px-2 py-2 text-ctl text-ink-muted"
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
                  : "assistant.noConversations",
              )}
            </p>
          )}
        </div>
        <p className="border-t border-line px-1 pt-2 text-note leading-relaxed text-ink-muted">
          {t("assistant.boundary")}
        </p>
      </aside>
      <section
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        aria-label={t("assistant.chat")}
      >
        <header className="flex shrink-0 items-center justify-between gap-3 border-b border-line px-4 py-2.5">
          <div className="flex min-w-0 items-center gap-1">
            <h1 className="min-w-0 truncate text-ui font-semibold">
              {session?.title || t("assistant.title")}
            </h1>
            {session && (
              <Button
                variant="icon"
                title={t("assistant.rename")}
                aria-label={t("assistant.rename")}
                onClick={() => setRename(session.title)}
              >
                <Pencil aria-hidden className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
          {Boolean(session || bootstrap?.models.length) && (
            <MenuSelect
              aria-label={t("assistant.model")}
              value={session?.modelId ?? assistant.newModelId}
              onChange={(modelId) =>
                session
                  ? void assistant.update({ modelId })
                  : assistant.chooseNewModel(modelId)
              }
              disabled={!writable}
              searchable
              searchPlaceholder={t("assistant.searchModels")}
              placeholder={session?.modelId || t("assistant.model")}
              width={310}
              align="end"
              triggerClassName="!max-w-[min(250px,100%)]"
              options={(bootstrap?.models ?? []).map((model) => ({
                value: model.id,
                label: model.name,
                note: [
                  model.contextWindow
                    ? t("assistant.contextWindow", {
                        count: model.contextWindow,
                      })
                    : "",
                  model.inputPricePerMillion
                    ? t("assistant.modelPrice", {
                        input: model.inputPricePerMillion,
                        output: model.outputPricePerMillion ?? "—",
                      })
                    : "",
                ]
                  .filter(Boolean)
                  .join(" · "),
              }))}
            />
          )}
        </header>
        {bootstrap?.modelUnavailableReason && (
          <p
            role="status"
            className="border-b border-line px-4 py-2 text-note text-ink-muted"
          >
            {t("assistant.modelUnavailable")}
          </p>
        )}
        {bootstrap && !bootstrap.runtime.available && (
          <div
            role="alert"
            className="border-b border-line bg-danger-soft px-4 py-3 text-ctl text-danger"
          >
            <p className="font-medium">{t("assistant.runtimeUnavailable")}</p>
            <p className="mt-1">{t("assistant.runtimeUnavailableHint")}</p>
            <Button
              variant="ghost"
              className="mt-1"
              onClick={() => void assistant.loadBootstrap()}
            >
              {t("common.retry")}
            </Button>
          </div>
        )}
        {assistant.error !== null && (
          <div
            role="alert"
            className="flex items-start justify-between gap-3 border-b border-line bg-danger-soft px-4 py-2 text-ctl text-danger"
          >
            <p className="break-words">
              {t(assistantErrorKey(assistant.error))}
            </p>
            <Button
              variant="icon"
              aria-label={t("common.close")}
              onClick={() => assistant.setError(null)}
            >
              <X aria-hidden className="h-3.5 w-3.5" />
            </Button>
          </div>
        )}
        <div
          ref={scroll}
          onScroll={() => {
            if (scroll.current)
              follow.current =
                scroll.current.scrollHeight -
                  scroll.current.scrollTop -
                  scroll.current.clientHeight <
                100;
          }}
          className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4"
          aria-busy={assistant.loading}
        >
          {assistant.loading ? (
            <div
              role="status"
              className="flex justify-center gap-2 py-12 text-ctl text-ink-muted"
            >
              <Loader2 aria-hidden className="h-4 w-4 animate-spin" />
              {t("assistant.loading")}
            </div>
          ) : (
            <>
              {!messages.length && !assistant.stream && (
                <div className="mx-auto flex min-h-full max-w-md flex-col items-center justify-center py-8 text-center">
                  <div className="mb-4 grid h-10 w-10 place-items-center rounded-card bg-brand-soft text-brand-ink">
                    <MessageSquare aria-hidden className="h-5 w-5" />
                  </div>
                  <h2 className="text-title font-semibold">
                    {t("assistant.welcome")}
                  </h2>
                  <p className="mt-2 max-w-sm text-ctl leading-relaxed text-ink-muted">
                    {t("assistant.description")}
                  </p>
                  {writable && !assistant.draft.trim() && (
                    <div className="mt-5 grid w-full max-w-[300px] gap-2">
                      {STARTERS.map((id) => (
                        <Button
                          key={id}
                          variant="secondary"
                          className="h-auto min-h-9 justify-between whitespace-normal px-3 py-2 text-left font-normal"
                          onClick={() => {
                            focusStarter.current = true;
                            assistant.setDraft(
                              t(`assistant.starters.${id}.prompt`),
                            );
                          }}
                        >
                          <span>{t(`assistant.starters.${id}.label`)}</span>
                          <ArrowUpRight
                            aria-hidden
                            className="h-3.5 w-3.5 shrink-0 text-ink-faint"
                          />
                        </Button>
                      ))}
                    </div>
                  )}
                  <p className="mt-4 text-note text-ink-muted">
                    {t("assistant.startHint")}
                  </p>
                </div>
              )}
              <div className="mx-auto max-w-[760px]">
                {messages.map((message) => (
                  <AssistantMessageView
                    key={message.id}
                    message={message}
                    disabled={!writable}
                    onEdit={(text) =>
                      assistant.send({ text, editMessageId: message.id })
                    }
                    onRegenerate={() =>
                      void assistant.send({
                        text: "",
                        regenerateMessageId: message.id,
                      })
                    }
                  />
                ))}
                {assistant.stream?.role === "assistant" && (
                  <AssistantMessageView
                    message={assistant.stream}
                    disabled
                    onEdit={async () => false}
                    onRegenerate={() => undefined}
                  />
                )}
                {assistant.compacting && (
                  <p role="status" className="py-2 text-note text-ink-muted">
                    {t("assistant.compacting")}
                  </p>
                )}
              </div>
            </>
          )}
        </div>
        <footer className="shrink-0 bg-card px-4 pb-3 pt-2">
          <div className="mx-auto w-full max-w-[760px]">
            <div className="rounded-list border border-line bg-card shadow-[0_1px_3px_rgba(0,0,0,.04)] transition-[border-color,box-shadow] focus-within:border-brand/60 focus-within:ring-2 focus-within:ring-brand/10">
              <Textarea
                ref={composer}
                rows={3}
                className="max-h-40 min-h-[76px] resize-none border-0 bg-transparent px-3 py-3 text-ui focus:ring-0"
                aria-label={t("assistant.message")}
                placeholder={t("assistant.composerPlaceholder")}
                value={assistant.draft}
                disabled={!available}
                readOnly={busy}
                onChange={(event) => assistant.setDraft(event.target.value)}
                onKeyDown={(event) => {
                  // WKWebView reports the Enter that confirms an IME candidate
                  // with isComposing false but keyCode 229.
                  if (
                    event.key === "Enter" &&
                    !event.shiftKey &&
                    !event.nativeEvent.isComposing &&
                    event.keyCode !== 229
                  ) {
                    event.preventDefault();
                    if (writable) send();
                  }
                }}
              />
              <div className="flex items-center justify-between gap-2 px-3 pb-2">
                <span className="min-w-0 truncate text-note text-ink-faint">
                  {run ? t("assistant.running") : t("assistant.sendHint")}
                </span>
                {run ? (
                  <Button variant="secondary" onClick={stop}>
                    <Square aria-hidden className="h-3.5 w-3.5" />
                    {t("assistant.stop")}
                  </Button>
                ) : (
                  <Button
                    aria-label={t("assistant.send")}
                    disabled={!writable || !assistant.draft.trim()}
                    onClick={send}
                  >
                    <ArrowUp aria-hidden className="h-3.5 w-3.5" />
                    {t("assistant.send")}
                  </Button>
                )}
              </div>
            </div>
            {bootstrap?.key.status === "missing" && (
              <p className="mt-2 text-note text-ink-muted">
                {t("assistant.keyMissingHint")}
              </p>
            )}
          </div>
        </footer>
      </section>
      <Dialog
        open={rename !== null}
        onOpenChange={(open) => {
          if (!open) setRename(null);
        }}
      >
        <DialogContent className="max-w-sm">
          <DialogHeader>
            <DialogTitle>{t("assistant.rename")}</DialogTitle>
            <DialogDescription>
              {t("assistant.renameDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="px-[18px] py-3">
            <Input
              autoFocus
              aria-label={t("assistant.conversationTitle")}
              value={rename ?? ""}
              maxLength={120}
              onChange={(event) => setRename(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === "Enter") saveTitle();
              }}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRename(null)}>
              {t("common.cancel")}
            </Button>
            <Button disabled={!rename?.trim()} onClick={saveTitle}>
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  );
}
