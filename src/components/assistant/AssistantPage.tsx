import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  ArchiveRestore,
  ArrowUp,
  ArrowUpRight,
  Brain,
  Download,
  Globe,
  Loader2,
  MessageSquare,
  Paperclip,
  Pencil,
  Plus,
  Search,
  Settings2,
  Shield,
  Square,
  Trash2,
  X,
} from "lucide-react";
import { assistantApi, type AssistantPermission } from "@/lib/api/assistant";
import { useAssistant } from "./useAssistant";
import { AssistantCapabilities } from "./AssistantCapabilities";
import { AssistantAuthorization } from "./AssistantAuthorization";
import { AssistantBackupDialog } from "./AssistantBackupDialog";
import { AssistantMessageView } from "./AssistantMessageView";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { MenuSelect } from "@/components/ui/menu-select";
import { Switch } from "@/components/ui/switch";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { useShell, useShellRefresh } from "@/components/shell/ShellContext";
import { cn } from "@/lib/utils";

const localizedErrorCodes = [
  "assistant_search_outcome_unknown",
  "assistant_operation_outcome_unknown",
  "assistant_vision_model_required",
  "assistant_signing_out",
  "assistant_shutting_down",
  "assistant_sign_in_required",
] as const;

interface AssistantPageProps {
  enabled?: boolean;
  onLoginRequested?: () => void;
}

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
  const { t } = useTranslation();
  const shell = useShell();
  const assistant = useAssistant(true);
  const errorCode = localizedErrorCodes.find((code) =>
    assistant.error?.includes(code),
  );
  const { detail, bootstrap, run, busy } = assistant;
  const [rename, setRename] = useState<string | null>(null);
  const [memoryOpen, setMemoryOpen] = useState(false);
  const [memoryText, setMemoryText] = useState("");
  const [memoryId, setMemoryId] = useState<string | undefined>();
  const [fullAccess, setFullAccess] = useState(false);
  const [purge, setPurge] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [notice, setNotice] = useState("");
  const scroll = useRef<HTMLDivElement>(null);
  const follow = useRef(true);
  const composer = useRef<HTMLTextAreaElement>(null);
  const focusStarter = useRef(false);
  const session = detail?.session;
  const archived = Boolean(session?.deletedAt);
  const available = Boolean(
    bootstrap?.runtime.available &&
      bootstrap.models.length > 0 &&
      !bootstrap.reauthorizationRequired &&
      assistant.connected,
  );
  const writable =
    available && !archived && !run && !busy && !assistant.loading;
  const permissionModes: AssistantPermission[] = [
    "readOnly",
    "ask",
    "autoConfig",
    "fullAccess",
  ];
  const reasonText = (
    scope: "runtimeReason" | "searchReason",
    reason: string | null | undefined,
    fallback: string,
  ) =>
    reason
      ? t(`assistant.${scope}.${reason}`, {
          defaultValue: /^[a-z][a-z0-9_]*$/.test(reason) ? t(fallback) : reason,
        })
      : t(fallback);

  useShellRefresh("assistant", async () => {
    await assistant.loadBootstrap();
    await assistant.refreshSessions();
    if (assistant.selectedId)
      await assistant.refreshDetail(assistant.selectedId);
  });

  useEffect(() => {
    setRename(null);
    setMemoryOpen(false);
    setMemoryText("");
    setMemoryId(undefined);
    setFullAccess(false);
    setPurge(false);
    setNotice("");
    follow.current = true;
  }, [assistant.selectedId]);
  useEffect(() => {
    if (follow.current && scroll.current)
      scroll.current.scrollTop = scroll.current.scrollHeight;
  }, [
    assistant.stream,
    detail?.messages.length,
    assistant.operations.length,
    assistant.approval,
  ]);
  useLayoutEffect(() => {
    if (!focusStarter.current) return;
    focusStarter.current = false;
    const input = composer.current;
    if (input && !input.disabled) {
      input.focus();
      input.setSelectionRange(input.value.length, input.value.length);
    }
  }, [assistant.draft]);

  const confirmPermission = async () => {
    setConfirming(true);
    try {
      await assistant.update({
        permissionMode: "fullAccess",
        fullAccessAcknowledged: true,
      });
      setFullAccess(false);
    } finally {
      setConfirming(false);
    }
  };
  const exportSession = async () => {
    if (!session) return;
    try {
      const result = await assistantApi.exportSession(session.id);
      if (result.path) setNotice(t("assistant.exported"));
    } catch (error) {
      assistant.showError(error);
    }
  };
  const purgeSession = async () => {
    if (!session) return;
    setConfirming(true);
    try {
      await assistantApi.purgeSession(session.id);
      assistant.forgetSession(session.id);
      setPurge(false);
      assistant.select(null);
      await assistant.refreshSessions();
    } catch (error) {
      assistant.showError(error);
    } finally {
      setConfirming(false);
    }
  };
  const send = () => {
    follow.current = true;
    void assistant.send().then((sent) => {
      if (sent) composer.current?.focus();
    });
  };

  return (
    <div className="flex h-full min-h-0 overflow-hidden rounded-list border border-line bg-card">
      <aside
        className="flex w-52 shrink-0 flex-col border-r border-line bg-card-2 p-3 max-[720px]:w-[170px]"
        aria-label={t("assistant.conversations")}
      >
        <MenuSelect
          aria-label={t("assistant.chooseAssistant")}
          value={assistant.kind}
          onChange={assistant.switchKind}
          triggerClassName="!h-8 !max-w-full w-full justify-between border-transparent bg-transparent pl-1 font-semibold text-ink"
          options={[
            {
              value: "general",
              icon: <MessageSquare aria-hidden />,
              label: t("assistant.general"),
              note: t("assistant.generalDescription"),
            },
            {
              value: "configuration",
              icon: <Settings2 aria-hidden />,
              label: t("assistant.configuration"),
              note: t("assistant.configurationDescription"),
            },
          ]}
        />
        <Button
          className="mb-3 mt-2 w-full"
          disabled={!available || busy}
          onClick={() => void assistant.create()}
        >
          <Plus className="h-3.5 w-3.5" />
          {t("assistant.newConversation")}
        </Button>
        <div className="relative">
          <Search className="pointer-events-none absolute left-2 top-1.5 h-3.5 w-3.5 text-ink-muted" />
          <Input
            className="pl-7"
            aria-label={t("assistant.search")}
            placeholder={t("assistant.search")}
            value={assistant.search}
            onChange={(event) => assistant.setSearch(event.target.value)}
          />
        </div>
        <div className="my-2 flex items-center justify-between text-note text-ink-muted">
          <span>
            {t(
              assistant.deleted ? "assistant.trash" : "assistant.conversations",
            )}
          </span>
          <Button
            variant="icon"
            aria-label={t(
              assistant.deleted
                ? "assistant.showActive"
                : "assistant.showTrash",
            )}
            title={t(
              assistant.deleted
                ? "assistant.showActive"
                : "assistant.showTrash",
            )}
            onClick={() => {
              assistant.select(null);
              assistant.setDeleted(!assistant.deleted);
            }}
          >
            <ArchiveRestore className="h-3.5 w-3.5" />
          </Button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">
          <ul className="space-y-1">
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
                      : new Date(item.updatedAt).toLocaleDateString()}
                  </span>
                </button>
              </li>
            ))}
          </ul>
          {!assistant.sessions.length && (
            <p className="px-2 py-6 text-center text-note text-ink-muted">
              {t(
                assistant.deleted
                  ? "assistant.emptyTrash"
                  : "assistant.noConversations",
              )}
            </p>
          )}
        </div>
        <div className="-mx-3 mt-2 border-t border-line px-2 pt-2">
          <AssistantBackupDialog
            onError={assistant.showError}
            onPrepare={assistant.flushDrafts}
            onRestored={async () => {
              await assistant.loadBootstrap();
              await assistant.refreshSessions();
              if (assistant.selectedId)
                await assistant.refreshDetail(assistant.selectedId);
            }}
          />
          {assistant.kind === "configuration" && bootstrap?.tools && (
            <AssistantCapabilities
              capabilities={bootstrap.tools}
              mode={session?.permissionMode ?? "ask"}
            />
          )}
          <p className="px-1 pt-2 text-note leading-relaxed text-ink-muted">
            {t(
              assistant.kind === "general"
                ? "assistant.generalBoundary"
                : "assistant.configurationBoundary",
            )}
          </p>
        </div>
      </aside>
      <section
        className="flex min-h-0 min-w-0 flex-1 flex-col"
        aria-label={t("assistant.chat")}
      >
        <header className="shrink-0 border-b border-line px-4 py-2.5">
          <div className="flex min-w-0 items-center justify-between gap-2">
            <h1 className="min-w-0 truncate text-ui font-semibold">
              {session?.title ||
                t(
                  assistant.kind === "general"
                    ? "assistant.general"
                    : "assistant.configuration",
                )}
            </h1>
            {session && (
              <div className="flex shrink-0 items-center gap-1">
                <Button
                  variant="icon"
                  title={t("assistant.rename")}
                  aria-label={t("assistant.rename")}
                  disabled={Boolean(run) || archived}
                  onClick={() => setRename(session.title)}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="icon"
                  title={t("assistant.export")}
                  aria-label={t("assistant.export")}
                  onClick={() => void exportSession()}
                >
                  <Download className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="icon"
                  title={t("assistant.memory")}
                  aria-label={t("assistant.memory")}
                  onClick={() => setMemoryOpen(true)}
                >
                  <Brain className="h-3.5 w-3.5" />
                </Button>
                {archived ? (
                  <>
                    <Button
                      variant="secondary"
                      onClick={() => void assistant.update({ deleted: false })}
                    >
                      <ArchiveRestore className="h-3.5 w-3.5" />
                      {t("assistant.restore")}
                    </Button>
                    <Button
                      variant="dangerGhost"
                      aria-label={t("assistant.purge")}
                      onClick={() => setPurge(true)}
                    >
                      <Trash2 className="h-3.5 w-3.5" />
                    </Button>
                  </>
                ) : (
                  <Button
                    variant="icon"
                    title={t("assistant.delete")}
                    aria-label={t("assistant.delete")}
                    disabled={Boolean(run)}
                    onClick={() => void assistant.update({ deleted: true })}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
            )}
          </div>
          {Boolean(session || bootstrap?.models.length) && (
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1.5">
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
              {session && assistant.kind === "configuration" && (
                <div className="flex items-center gap-1">
                  <Shield className="h-3.5 w-3.5 text-ink-muted" />
                  <MenuSelect
                    aria-label={t("assistant.permissions")}
                    value={session.permissionMode}
                    onChange={(permissionMode) =>
                      permissionMode === "fullAccess"
                        ? setFullAccess(true)
                        : void assistant.update({ permissionMode })
                    }
                    disabled={
                      !available || archived || busy || assistant.loading
                    }
                    width={300}
                    options={permissionModes.map((mode) => ({
                      value: mode,
                      label: t(`assistant.permission.${mode}`),
                      note: t(`assistant.permissionDescription.${mode}`),
                    }))}
                  />
                </div>
              )}
              {session && (
                <>
                  <label className="flex cursor-pointer items-center gap-1.5 text-ctl text-ink-ctl">
                    <Globe className="h-3.5 w-3.5" />
                    {t("assistant.web")}
                    <Switch
                      aria-label={t("assistant.web")}
                      checked={session.webSearch}
                      disabled={!writable || !bootstrap?.search?.available}
                      onCheckedChange={(webSearch) =>
                        void assistant.update({ webSearch })
                      }
                    />
                  </label>
                  <span className="text-note text-ink-muted">
                    {!bootstrap?.search?.available
                      ? reasonText(
                          "searchReason",
                          bootstrap?.search?.reason,
                          "assistant.searchUnavailable",
                        )
                      : bootstrap.search.unitPrice != null &&
                          bootstrap.search.currency
                        ? t("assistant.searchPrice", {
                            currency: bootstrap.search.currency,
                            price: bootstrap.search.unitPrice,
                          })
                        : t("assistant.searchPriceUnknown")}
                  </span>
                </>
              )}
            </div>
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
        {bootstrap?.reauthorizationRequired && (
          <p
            role="status"
            className="border-b border-line bg-danger-soft px-4 py-3 text-ctl text-danger"
          >
            {t("assistant.reauthorizationRequired")}
            <AssistantAuthorization onAuthorized={assistant.loadBootstrap} />
          </p>
        )}
        {bootstrap && !bootstrap.runtime.available && (
          <div
            role="status"
            className="border-b border-line bg-danger-soft px-4 py-3 text-ctl text-danger"
          >
            <p className="font-medium">{t("assistant.runtimeUnavailable")}</p>
            <p className="mt-1 whitespace-pre-wrap break-words">
              {reasonText(
                "runtimeReason",
                bootstrap.runtime.reason,
                "assistant.runtimeUnavailableHint",
              )}
            </p>
            <Button
              variant="ghost"
              className="mt-1"
              onClick={() => void assistant.loadBootstrap()}
            >
              {t("assistant.retry")}
            </Button>
          </div>
        )}
        {assistant.error && (
          <div
            role="alert"
            className="flex items-start justify-between gap-3 border-b border-line bg-danger-soft px-4 py-2 text-ctl text-danger"
          >
            <p className="whitespace-pre-wrap break-words">
              {t(errorCode ?? assistant.error, {
                defaultValue: assistant.error,
              })}
            </p>
            <Button
              variant="icon"
              aria-label={t("common.close")}
              onClick={() => assistant.setError(null)}
            >
              <X className="h-3.5 w-3.5" />
            </Button>
          </div>
        )}
        {notice && (
          <p role="status" className="px-4 pt-2 text-note text-ink-muted">
            {notice}
          </p>
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
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("assistant.loading")}
            </div>
          ) : (
            <>
              {!detail?.messages.length && !assistant.stream && (
                <div className="mx-auto flex min-h-full max-w-md flex-col items-center justify-center py-8 text-center">
                  <div className="mb-4 grid h-10 w-10 place-items-center rounded-card bg-brand-soft text-brand-ink">
                    {assistant.kind === "general" ? (
                      <MessageSquare aria-hidden className="h-5 w-5" />
                    ) : (
                      <Settings2 aria-hidden className="h-5 w-5" />
                    )}
                  </div>
                  <h2 className="text-title font-semibold">
                    {t(
                      assistant.kind === "general"
                        ? "assistant.generalWelcome"
                        : "assistant.configurationWelcome",
                    )}
                  </h2>
                  <p className="mt-2 max-w-sm text-ctl leading-relaxed text-ink-muted">
                    {t(
                      assistant.kind === "general"
                        ? "assistant.generalDescription"
                        : "assistant.configurationDescription",
                    )}
                  </p>
                  {writable &&
                    !assistant.draft.trim() &&
                    !assistant.attachments.length && (
                      <div className="mt-5 grid w-full max-w-[300px] gap-2">
                        {(assistant.kind === "general"
                          ? ["summary", "writing", "explain"]
                          : ["status", "connection", "model"]
                        ).map((id) => (
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
                {detail?.messages.map((message) => (
                  <AssistantMessageView
                    key={message.id}
                    message={message}
                    disabled={!writable}
                    onError={assistant.showError}
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
                {assistant.stream && (
                  <AssistantMessageView
                    message={assistant.stream}
                    disabled
                    onError={assistant.showError}
                    onEdit={async () => false}
                    onRegenerate={() => undefined}
                  />
                )}
              </div>
              {assistant.compacting && (
                <p role="status" className="py-2 text-note text-ink-muted">
                  {t("assistant.compacting")}
                </p>
              )}
              {!!assistant.operations.length && (
                <details className="mx-auto my-3 max-w-[850px] rounded-ctl border border-line px-3 py-2">
                  <summary className="cursor-pointer text-ctl text-ink-muted">
                    {t("assistant.operations", {
                      count: assistant.operations.length,
                    })}
                  </summary>
                  <ul className="mt-2 space-y-2">
                    {assistant.operations.map((operation) => (
                      <li
                        key={operation.id}
                        className="rounded-ctl bg-card-2 p-2 text-ctl"
                      >
                        <div className="flex items-center gap-2">
                          <span
                            className={
                              operation.status === "failed"
                                ? "text-danger"
                                : "text-ink-muted"
                            }
                          >
                            {t(`assistant.operationStatus.${operation.status}`)}
                          </span>
                          <span className="font-medium">
                            {t(
                              `assistant.operationTitle.${operation.toolName ?? ""}`,
                              { defaultValue: operation.title },
                            )}
                          </span>
                        </div>
                        {operation.summary && (
                          <p className="mt-1 text-ink-muted">
                            {operation.summary}
                          </p>
                        )}
                        {operation.command && (
                          <pre className="mt-2 overflow-x-auto whitespace-pre-wrap break-all text-note">
                            {operation.command}
                          </pre>
                        )}
                        {operation.output && (
                          <pre className="mt-2 max-h-60 overflow-auto whitespace-pre-wrap break-all text-note">
                            {operation.output}
                          </pre>
                        )}
                        {operation.action === "openTools" && (
                          <Button
                            className="mt-2"
                            variant="secondary"
                            onClick={() => shell.setActive("tools")}
                          >
                            {t("assistant.openTools")}
                          </Button>
                        )}
                      </li>
                    ))}
                  </ul>
                </details>
              )}
              {assistant.approval && (
                <div
                  role="region"
                  aria-label={t("assistant.approval")}
                  className="mx-auto my-4 max-w-[850px] rounded-list border border-brand bg-brand-soft p-4"
                >
                  <div className="flex items-center gap-2 text-ui font-semibold">
                    <Shield className="h-4 w-4" />
                    {t(`assistant.operationTitle.${assistant.approval.title}`, {
                      defaultValue: assistant.approval.title,
                    })}
                  </div>
                  <p className="mt-2 whitespace-pre-wrap text-ctl">
                    {t(
                      `assistant.operationTitle.${assistant.approval.description}`,
                      { defaultValue: assistant.approval.description },
                    )}
                  </p>
                  {assistant.approval.details && (
                    <pre className="mt-2 max-h-52 overflow-auto whitespace-pre-wrap break-all rounded-ctl bg-card p-2 text-note">
                      {assistant.approval.details}
                    </pre>
                  )}
                  <div className="mt-3 flex flex-wrap gap-2">
                    <Button
                      onClick={() =>
                        void assistant.respondApproval("allowOnce")
                      }
                    >
                      {t("assistant.allowOnce")}
                    </Button>
                    <Button
                      variant="secondary"
                      onClick={() =>
                        void assistant.respondApproval("allowSession")
                      }
                    >
                      {t("assistant.allowSession")}
                    </Button>
                    <Button
                      variant="ghost"
                      onClick={() => void assistant.respondApproval("deny")}
                    >
                      {t("assistant.deny")}
                    </Button>
                  </div>
                </div>
              )}
            </>
          )}
        </div>
        <footer className="shrink-0 bg-card px-4 pb-3 pt-2">
          <div className="mx-auto w-full max-w-[760px]">
            {archived ? (
              <p className="py-3 text-center text-ctl text-ink-muted">
                {t("assistant.archivedHint")}
              </p>
            ) : (
              <div className="rounded-list border border-line bg-card shadow-[0_1px_3px_rgba(0,0,0,.04)] transition-[border-color,box-shadow] focus-within:border-brand/60 focus-within:ring-2 focus-within:ring-brand/10">
                {!!assistant.attachments.length && (
                  <ul
                    aria-label={t("assistant.attachments")}
                    className="flex flex-wrap gap-2 px-3 pt-3"
                  >
                    {assistant.attachments.map((attachment) => (
                      <li
                        key={attachment.id}
                        className="flex max-w-full items-center gap-1 rounded-ctl border border-line bg-card-2 pl-2 text-ctl"
                      >
                        <span className="truncate">{attachment.name}</span>
                        <span
                          className={
                            attachment.status === "error"
                              ? "text-danger"
                              : "text-ink-muted"
                          }
                        >
                          {attachment.reason ||
                            (attachment.status === "processing"
                              ? t("assistant.processing")
                              : "")}
                        </span>
                        <Button
                          variant="icon"
                          aria-label={t("assistant.removeAttachment", {
                            name: attachment.name,
                          })}
                          disabled={Boolean(run)}
                          onClick={() =>
                            assistant.setAttachments((items) =>
                              items.filter((item) => item.id !== attachment.id),
                            )
                          }
                        >
                          <X className="h-3 w-3" />
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
                <Textarea
                  ref={composer}
                  rows={3}
                  className="min-h-[76px] max-h-40 resize-none border-0 bg-transparent px-3 py-3 text-ui focus:ring-0"
                  aria-label={t("assistant.message")}
                  placeholder={t("assistant.composerPlaceholder")}
                  value={assistant.draft}
                  disabled={!available || busy}
                  onChange={(event) => assistant.setDraft(event.target.value)}
                  onKeyDown={(event) => {
                    if (
                      event.key === "Enter" &&
                      !event.shiftKey &&
                      !event.nativeEvent.isComposing
                    ) {
                      event.preventDefault();
                      if (writable) send();
                    }
                  }}
                />
                <div className="flex items-center justify-between gap-2 px-2 pb-2">
                  <div className="flex min-w-0 items-center gap-2">
                    <Button
                      variant="ghost"
                      disabled={!writable}
                      onClick={() => void assistant.pickAttachments()}
                    >
                      <Paperclip className="h-3.5 w-3.5" />
                      {t("assistant.attach")}
                    </Button>
                    <span className="truncate text-note text-ink-faint">
                      {run ? t("assistant.running") : t("assistant.sendHint")}
                    </span>
                  </div>
                  {run ? (
                    <Button
                      variant="secondary"
                      onClick={() => void assistant.stop()}
                    >
                      <Square className="h-3.5 w-3.5" />
                      {t("assistant.stop")}
                    </Button>
                  ) : (
                    <Button
                      aria-label={t("assistant.send")}
                      disabled={
                        !writable ||
                        (!assistant.draft.trim() &&
                          !assistant.attachments.length) ||
                        assistant.attachments.some(
                          (item) => item.status !== "ready",
                        )
                      }
                      onClick={send}
                    >
                      <ArrowUp className="h-3.5 w-3.5" />
                      {t("assistant.send")}
                    </Button>
                  )}
                </div>
              </div>
            )}
            {detail?.usage && (
              <p className="mt-2 text-right text-note text-ink-muted">
                {t("assistant.usage", {
                  input: detail.usage.inputTokens ?? "—",
                  output: detail.usage.outputTokens ?? "—",
                })}
                {detail.usage.estimatedCostUsd &&
                  ` · ${t("assistant.estimatedCost", { cost: detail.usage.estimatedCostUsd })}`}
                {detail.usage.billedCostUsd &&
                  ` · ${t("assistant.billedCost", { cost: detail.usage.billedCostUsd })}`}
                {detail.usage.usageCoverage !== "complete" && (
                  <span className="block">
                    {t("assistant.conversationUsageIncomplete")}
                  </span>
                )}
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
                if (event.key === "Enter" && rename?.trim())
                  void assistant
                    .update({ title: rename.trim() })
                    .then(() => setRename(null));
              }}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setRename(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              disabled={!rename?.trim()}
              onClick={() =>
                void assistant
                  .update({ title: rename?.trim() })
                  .then(() => setRename(null))
              }
            >
              {t("common.save")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <Dialog open={memoryOpen} onOpenChange={setMemoryOpen}>
        <DialogContent className="max-w-lg">
          <DialogHeader>
            <DialogTitle>{t("assistant.memory")}</DialogTitle>
            <DialogDescription>
              {t("assistant.memoryDescription")}
            </DialogDescription>
          </DialogHeader>
          <div className="max-h-[55vh] space-y-3 overflow-y-auto px-[18px] py-3">
            {detail?.memories.map((memory) => (
              <div
                key={memory.id}
                className="flex items-start gap-2 rounded-ctl border border-line p-2"
              >
                <p className="min-w-0 flex-1 whitespace-pre-wrap break-words text-ctl">
                  {memory.content}
                </p>
                <Button
                  variant="icon"
                  aria-label={t("assistant.editMemory")}
                  disabled={archived || Boolean(run)}
                  onClick={() => {
                    setMemoryText(memory.content);
                    setMemoryId(memory.id);
                  }}
                >
                  <Pencil className="h-3.5 w-3.5" />
                </Button>
                <Button
                  variant="icon"
                  aria-label={t("assistant.deleteMemory")}
                  disabled={archived || Boolean(run)}
                  onClick={() => void assistant.deleteMemory(memory.id)}
                >
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            ))}
            {!detail?.memories.length && (
              <p className="text-ctl text-ink-muted">
                {t("assistant.noMemories")}
              </p>
            )}
            <Textarea
              aria-label={t("assistant.memoryContent")}
              placeholder={t("assistant.memoryPlaceholder")}
              value={memoryText}
              disabled={archived || Boolean(run)}
              maxLength={4000}
              onChange={(event) => setMemoryText(event.target.value)}
            />
          </div>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setMemoryOpen(false)}>
              {t("common.close")}
            </Button>
            <Button
              disabled={archived || Boolean(run) || !memoryText.trim()}
              onClick={() =>
                void assistant
                  .saveMemory(memoryText.trim(), memoryId)
                  .then((saved) => {
                    if (saved) {
                      setMemoryText("");
                      setMemoryId(undefined);
                    }
                  })
              }
            >
              {t(memoryId ? "common.save" : "assistant.addMemory")}
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <ConfirmDialog
        isOpen={fullAccess}
        title={t("assistant.fullAccessTitle")}
        message={t("assistant.fullAccessWarning")}
        confirmText={t("assistant.enableFullAccess")}
        busy={confirming}
        onConfirm={() => void confirmPermission()}
        onCancel={() => setFullAccess(false)}
      />
      <ConfirmDialog
        isOpen={purge}
        title={t("assistant.purge")}
        message={t("assistant.purgeWarning")}
        confirmText={t("assistant.purge")}
        busy={confirming}
        onConfirm={() => void purgeSession()}
        onCancel={() => setPurge(false)}
      />
    </div>
  );
}
