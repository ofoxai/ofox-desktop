import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { MenuSelect } from "@/components/ui/menu-select";
import {
  assistantApi,
  assistantErrorKey,
  type AssistantAudioResult,
  type AssistantModel,
  type AssistantPasteTarget,
  type AssistantRunRef,
  type AssistantSpeechPreferences,
} from "@/lib/api/assistant";
import { AssistantAudioPlayer } from "./AssistantAudioPlayer";
export interface AssistantSpeechOutputProps {
  target: AssistantPasteTarget;
  models: readonly AssistantModel[];
  rememberedModel?: string | null;
  speechPreferences?: AssistantSpeechPreferences;
  initialText: string;
  sourceMessageId?: string;
  history?: readonly AssistantAudioResult[];
  onClose: () => void;
}
const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumSignificantDigits: 3,
});
export function AssistantSpeechOutput({
  target,
  models,
  rememberedModel,
  speechPreferences = {},
  initialText,
  sourceMessageId,
  history = [],
  onClose,
}: AssistantSpeechOutputProps) {
  const { t } = useTranslation();
  const ids = useId();
  const pendingModels = useRef<readonly AssistantModel[] | null>(null);
  const available = (pendingModels.current ?? models).filter(
    (m) => m.kind === "speech" && m.audio,
  );
  const initial =
    available.find((m) => m.id === rememberedModel) ?? available[0];
  const choices = useRef<AssistantSpeechPreferences>({ ...speechPreferences });
  const choice = (m: AssistantModel | undefined) => {
    const saved = m ? choices.current[m.id] : undefined;
    const validVoice =
      m?.audio?.voiceKind === "id" ? /^[A-Za-z0-9]{20}$/ : /^[A-Za-z]{1,64}$/;
    return {
      voice:
        saved?.voice && validVoice.test(saved.voice)
          ? saved.voice
          : (m?.audio?.suggestedVoices[0] ?? ""),
      format:
        saved?.format &&
        saved.format !== "pcm" &&
        m?.audio?.responseFormats.includes(saved.format)
          ? saved.format
          : (m?.audio?.defaultFormat ?? "wav"),
    };
  };
  const [modelId, setModelId] = useState(initial?.id ?? "");
  const model = available.find((m) => m.id === modelId);
  const [voice, setVoice] = useState(() => choice(initial).voice);
  const [format, setFormat] = useState(() => choice(initial).format);
  const [language, setLanguage] = useState("auto");
  const [text, setText] = useState(initialText);
  const [source, setSource] = useState(sourceMessageId);
  const [instructions, setInstructions] = useState("");
  const [busy, setBusy] = useState(false);
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<AssistantAudioResult | null>(null);
  const request = useRef<AssistantRunRef | null>(null);
  const stopSent = useRef(false);
  const generation = useRef(0);
  const release = () => {
    generation.current++;
    pendingModels.current = null;
    const active = request.current;
    if (active && !stopSent.current) {
      stopSent.current = true;
      void assistantApi
        .stop(target.sessionId, active.runId, active.operationId)
        .catch(() => undefined);
    }
    request.current = null;
  };
  useEffect(() => {
    generation.current++;
    return release;
    // The parent keys the dialog by owner, generation, session and selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.expectedOwner, target.expectedGeneration, target.sessionId]);
  const changeModel = (id: string) => {
    const next = available.find((m) => m.id === id);
    choices.current[modelId] = { voice, format };
    setModelId(id);
    setVoice(choice(next).voice);
    setFormat(choice(next).format);
    setInstructions("");
    setError(null);
    setResult(null);
  };
  const count = Array.from(text).length;
  const styleCount = Array.from(instructions).length;
  const validVoice =
    model?.audio?.voiceKind === "id"
      ? /^[A-Za-z0-9]{20}$/.test(voice)
      : /^[A-Za-z]{1,64}$/.test(voice);
  const valid = Boolean(
    model &&
      text.trim() &&
      count <= 4000 &&
      styleCount <= 1000 &&
      validVoice &&
      model.audio?.responseFormats.includes(format) &&
      format !== "pcm",
  );
  const price = model?.audio?.pricing;
  const rate =
    price?.inputTextPerCharacter == null
      ? NaN
      : Number(price.inputTextPerCharacter);
  const savedEstimate =
    result?.audio.estimate == null ? NaN : Number(result.audio.estimate);
  const estimate = result
    ? Number.isFinite(savedEstimate) && savedEstimate >= 0
      ? savedEstimate
      : null
    : price?.unit === "characters" && Number.isFinite(rate) && rate >= 0
      ? rate * count
      : null;
  const selectResult = (item: AssistantAudioResult) => {
    const params = item.params ?? {};
    setText(item.audio.text ?? "");
    setSource(undefined);
    setModelId(item.model);
    setVoice(typeof params.voice === "string" ? params.voice : "");
    setFormat(
      typeof params.responseFormat === "string" ? params.responseFormat : "",
    );
    setLanguage(
      typeof params.language === "string" &&
        ["zh", "en", "ja"].includes(params.language)
        ? params.language
        : "auto",
    );
    setInstructions(
      typeof params.instructions === "string" ? params.instructions : "",
    );
    setError(null);
    setResult(item);
  };
  const generate = async () => {
    if (!valid || request.current) return;
    const epoch = generation.current;
    pendingModels.current = models;
    const active = {
      runId: crypto.randomUUID(),
      operationId: crypto.randomUUID(),
    };
    request.current = active;
    stopSent.current = false;
    setStopping(false);
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      const value = await assistantApi.synthesize({
        ...target,
        ...active,
        modelId,
        input: text,
        voice,
        responseFormat: format,
        ...(language !== "auto"
          ? { language: language as "zh" | "en" | "ja" }
          : {}),
        ...(model?.audio?.instructions && instructions.trim()
          ? { instructions }
          : {}),
        ...(source ? { sourceMessageId: source } : {}),
      });
      if (epoch === generation.current) setResult(value);
    } catch (reason) {
      if (epoch === generation.current) setError(assistantErrorKey(reason));
    } finally {
      if (epoch === generation.current) {
        request.current = null;
        pendingModels.current = null;
        setBusy(false);
      }
    }
  };
  const stop = () => {
    const active = request.current;
    if (!active || stopSent.current) return;
    stopSent.current = true;
    setStopping(true);
    void assistantApi
      .stop(target.sessionId, active.runId, active.operationId)
      .catch((reason) => {
        if (request.current?.runId === active.runId) {
          stopSent.current = false;
          setStopping(false);
          setError(assistantErrorKey(reason));
        }
      });
  };
  const close = () => {
    release();
    onClose();
  };
  return (
    <Dialog
      open
      onOpenChange={(open) => {
        if (!open) close();
      }}
    >
      <DialogContent className="max-w-[560px]" zIndex="nested">
        <DialogHeader>
          <DialogTitle>{t("assistant.speech.title")}</DialogTitle>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-[18px] py-4">
          <DialogDescription className="text-note text-ink-muted">
            {t("assistant.speech.hint")}
          </DialogDescription>
          <label htmlFor={`${ids}-text`} className="text-ctl text-ink">
            {t("assistant.speech.text")}
          </label>
          <Textarea
            id={`${ids}-text`}
            rows={5}
            disabled={busy}
            value={text}
            onChange={(e) => {
              setText(e.target.value);
              setSource(undefined);
              setResult(null);
            }}
            className="min-h-[100px] resize-y"
          />
          <p
            className={
              count > 4000
                ? "text-note text-danger"
                : "text-note text-ink-muted"
            }
          >
            {t("assistant.speech.length", { count, limit: 4000 })}
          </p>
          <div className="flex flex-wrap items-end gap-3">
            <div className="flex min-w-0 flex-col gap-1.5">
              <label
                htmlFor={`${ids}-model`}
                className="text-note text-ink-muted"
              >
                {t("assistant.speech.model")}
              </label>
              <MenuSelect
                id={`${ids}-model`}
                aria-label={t("assistant.speech.model")}
                value={modelId}
                onChange={changeModel}
                disabled={busy}
                options={available.map((m) => ({ value: m.id, label: m.name }))}
                searchable
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label
                htmlFor={`${ids}-format`}
                className="text-note text-ink-muted"
              >
                {t("assistant.speech.format")}
              </label>
              <MenuSelect
                id={`${ids}-format`}
                aria-label={t("assistant.speech.format")}
                value={format}
                onChange={(value) => {
                  setFormat(value);
                  setResult(null);
                }}
                disabled={busy}
                options={(model?.audio?.responseFormats ?? [])
                  .filter((f) => f !== "pcm")
                  .map((f) => ({ value: f, label: f.toUpperCase() }))}
              />
            </div>
            <div className="flex flex-col gap-1.5">
              <label
                htmlFor={`${ids}-language`}
                className="text-note text-ink-muted"
              >
                {t("assistant.audio.language")}
              </label>
              <MenuSelect
                id={`${ids}-language`}
                aria-label={t("assistant.audio.language")}
                value={language}
                onChange={(value) => {
                  setLanguage(value);
                  setResult(null);
                }}
                disabled={busy}
                options={["auto", "zh", "en", "ja"].map((value) => ({
                  value,
                  label: t(`assistant.audio.${value}`),
                }))}
              />
            </div>
          </div>
          <label htmlFor={`${ids}-voice`} className="text-note text-ink-muted">
            {t(
              model?.audio?.voiceKind === "id"
                ? "assistant.speech.voiceId"
                : "assistant.speech.voiceName",
            )}
          </label>
          <Input
            id={`${ids}-voice`}
            value={voice}
            disabled={busy}
            autoComplete="off"
            onChange={(e) => {
              setVoice(e.target.value);
              setResult(null);
            }}
          />
          <p className="text-note text-ink-muted">
            {t(
              model?.audio?.voiceKind === "id"
                ? "assistant.speech.voiceIdHint"
                : "assistant.speech.voiceNameHint",
            )}
          </p>
          {model?.audio?.instructions && (
            <>
              <label
                htmlFor={`${ids}-style`}
                className="text-note text-ink-muted"
              >
                {t("assistant.speech.style")}
              </label>
              <Textarea
                id={`${ids}-style`}
                value={instructions}
                disabled={busy}
                rows={2}
                onChange={(e) => {
                  setInstructions(e.target.value);
                  setResult(null);
                }}
              />
              <p
                className={
                  styleCount > 1000
                    ? "text-note text-danger"
                    : "text-note text-ink-muted"
                }
              >
                {t("assistant.speech.styleLength", {
                  count: styleCount,
                  limit: 1000,
                })}
              </p>
            </>
          )}
          <p className="text-note text-ink-muted">
            {estimate === null
              ? t("assistant.speech.unknownEstimate")
              : t("assistant.speech.estimate", {
                  amount: usd.format(estimate),
                })}
          </p>
          {busy && (
            <div
              role="status"
              className="flex flex-col items-start gap-2 text-note text-ink-muted"
            >
              <span className="flex items-center gap-2">
                <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
                {t("assistant.speech.generating")}
              </span>
              <p>{t("assistant.audio.submittedHint")}</p>
              <Button variant="secondary" disabled={stopping} onClick={stop}>
                {t(
                  stopping
                    ? "assistant.audio.stopping"
                    : "assistant.speech.stop",
                )}
              </Button>
            </div>
          )}
          {result && (
            <div className="flex flex-col gap-1 text-note text-ink-muted">
              {result.status === "completed" ? (
                <>
                  <p role="status" className="text-ctl text-ink">
                    {t("assistant.speech.generated")}
                  </p>
                  {result.audio.output && (
                    <p>
                      {t("assistant.speech.outputInfo", {
                        seconds: (
                          result.audio.output.durationMs / 1000
                        ).toFixed(2),
                        size: (result.audio.output.sizeBytes / 1024).toFixed(2),
                      })}
                    </p>
                  )}
                  {result.outputs?.[0] && result.audio.output && (
                    <AssistantAudioPlayer
                      key={`${result.operationId}:${result.outputs[0]}`}
                      target={target}
                      attachmentId={result.outputs[0]}
                      mimeType={result.audio.output.mimeType}
                      durationMs={result.audio.output.durationMs}
                    />
                  )}
                  <p>
                    {result.cost == null
                      ? t("assistant.speech.unknownCost")
                      : t("assistant.audio.cost", {
                          amount: usd.format(Number(result.cost)),
                        })}
                  </p>
                </>
              ) : (
                <p role="alert" className="text-danger">
                  {t(
                    assistantErrorKey(
                      result.audio.errorCode ??
                        "assistant_audio_outcome_unknown",
                    ),
                  )}
                </p>
              )}
            </div>
          )}
          {history.filter((r) => r.kind === "speech").length > 0 && (
            <div className="flex flex-col gap-1">
              <p className="text-note text-ink-muted">
                {t("assistant.speech.history")}
              </p>
              <div className="max-h-[90px] overflow-y-auto">
                {[...history]
                  .reverse()
                  .filter((r) => r.kind === "speech")
                  .map((item, index) => (
                    <Button
                      key={item.operationId}
                      disabled={busy}
                      variant="ghost"
                      className="h-auto w-full justify-start truncate py-1.5 text-note"
                      onClick={() => selectResult(item)}
                    >
                      {t("assistant.speech.historyItem", { number: index + 1 })}
                    </Button>
                  ))}
              </div>
            </div>
          )}
          {error && (
            <p role="alert" className="text-note text-danger">
              {t(error)}
            </p>
          )}
        </div>
        <DialogFooter>
          <Button variant="ghost" onClick={close}>
            {t("assistant.audio.close")}
          </Button>
          <Button
            disabled={!valid || busy || Boolean(result)}
            onClick={() => void generate()}
          >
            {t("assistant.speech.generate")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
