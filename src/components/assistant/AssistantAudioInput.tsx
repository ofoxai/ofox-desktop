import { useEffect, useId, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Loader2, Mic, Square, Upload } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { AssistantAudioPlayer } from "./AssistantAudioPlayer";
import { Button } from "@/components/ui/button";
import { MenuSelect } from "@/components/ui/menu-select";
import { Textarea } from "@/components/ui/textarea";
import {
  assistantApi,
  assistantErrorKey,
  type AssistantAudioInput as AudioInput,
  type AssistantAudioResult,
  type AssistantModel,
  type AssistantPasteTarget,
  type AssistantRunRef,
} from "@/lib/api/assistant";
import { startRecording, type Recording } from "@/lib/assistantRecorder";

export interface AssistantAudioInputProps {
  target: AssistantPasteTarget;
  models: readonly AssistantModel[];
  rememberedModel?: string | null;
  history?: readonly AssistantAudioResult[];
  onInsert: (text: string) => boolean;
  onClose: () => void;
}
type Phase =
  | "idle"
  | "picking"
  | "starting"
  | "recording"
  | "stopping"
  | "transcribing";
const usd = new Intl.NumberFormat("en-US", {
  style: "currency",
  currency: "USD",
  maximumSignificantDigits: 3,
});
const NO_HISTORY: readonly AssistantAudioResult[] = [];

/** Explicit review and insertion preserve every existing composer draft. */
export function AssistantAudioInput({
  target,
  models,
  rememberedModel,
  history = NO_HISTORY,
  onInsert,
  onClose,
}: AssistantAudioInputProps) {
  const { t, i18n } = useTranslation();
  const ids = useId();
  const pendingModels = useRef<readonly AssistantModel[] | null>(null);
  const options = (pendingModels.current ?? models).filter(
    (model) => model.kind === "transcription" && model.audio,
  );
  const [modelId, setModelId] = useState(
    () =>
      options.find((model) => model.id === rememberedModel)?.id ??
      options[0]?.id ??
      "",
  );
  const [keepOriginal, setKeepOriginal] = useState(false);
  const [language, setLanguage] = useState("auto");
  const [input, setInput] = useState<AudioInput | null>(null);
  const inputNow = useRef<AudioInput | null>(null);
  const [phase, setPhaseState] = useState<Phase>("idle");
  const phaseNow = useRef<Phase>("idle");
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<AssistantAudioResult | null>(null);
  const [text, setText] = useState("");
  const [seconds, setSeconds] = useState(0);
  const [limited, setLimited] = useState(false);
  const [stopping, setStopping] = useState(false);
  const generation = useRef(0);
  const recording = useRef<Recording | null>(null);
  const permission = useRef<AbortController | null>(null);
  const request = useRef<AssistantRunRef | null>(null);
  const stopSent = useRef(false);
  const setPhase = (value: Phase) => {
    phaseNow.current = value;
    setPhaseState(value);
  };
  const discard = (file: AudioInput) => {
    void assistantApi
      .discardAudio({ ...target, inputId: file.inputId })
      .catch(() => undefined);
  };
  const release = () => {
    generation.current++;
    pendingModels.current = null;
    permission.current?.abort();
    permission.current = null;
    recording.current?.cancel();
    recording.current = null;
    if (inputNow.current) discard(inputNow.current);
    inputNow.current = null;
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
    // The parent keys this component by owner, generation and selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [target.expectedOwner, target.expectedGeneration, target.sessionId]);
  const currentModel = options.find((model) => model.id === modelId);
  const busy = phase !== "idle";
  const own = (epoch: number) => epoch === generation.current;
  const changeInput = (file: AudioInput) => {
    inputNow.current = file;
    setInput(file);
    setResult(null);
    setText("");
  };
  const selectResult = (value: AssistantAudioResult) => {
    setResult(value);
    setText(value.status === "completed" ? (value.audio.text ?? "") : "");
    setError(null);
  };
  const importFile = async () => {
    if (phaseNow.current !== "idle") return;
    const epoch = generation.current;
    setPhase("picking");
    setError(null);
    setLimited(false);
    try {
      const file = await assistantApi.pickAudio(target);
      if (!own(epoch)) {
        if (file) discard(file);
        return;
      }
      if (file) changeInput(file);
    } catch (reason) {
      if (own(epoch)) setError(assistantErrorKey(reason));
    } finally {
      if (own(epoch)) setPhase("idle");
    }
  };
  const importRecording = async (
    file: File,
    epoch: number,
    atLimit: boolean,
  ) => {
    if (!own(epoch)) return;
    setPhase("stopping");
    try {
      const imported = await assistantApi.importAudio(file, target);
      if (!own(epoch)) {
        discard(imported);
        return;
      }
      changeInput(imported);
      setLimited(atLimit);
    } catch (reason) {
      if (own(epoch)) setError(assistantErrorKey(reason));
    } finally {
      if (own(epoch)) {
        recording.current = null;
        setPhase("idle");
      }
    }
  };
  const record = async () => {
    if (phaseNow.current !== "idle") return;
    const epoch = generation.current;
    setPhase("starting");
    setError(null);
    setLimited(false);
    setSeconds(0);
    const controller = new AbortController();
    permission.current = controller;
    try {
      const value = await startRecording(
        {
          progress: (value) => {
            if (own(epoch)) setSeconds(value);
          },
          limit: (file) => {
            if (own(epoch)) void importRecording(file, epoch, true);
          },
          error: (code) => {
            if (own(epoch)) {
              setError(assistantErrorKey(code));
              recording.current = null;
              setPhase("idle");
            }
          },
        },
        controller.signal,
      );
      if (!own(epoch) || controller.signal.aborted) {
        value.cancel();
        return;
      }
      recording.current = value;
      setPhase("recording");
    } catch (reason) {
      if (own(epoch)) {
        if (!(reason instanceof DOMException && reason.name === "AbortError"))
          setError(assistantErrorKey(reason));
        setPhase("idle");
      }
    }
  };
  const finishRecording = async () => {
    const value = recording.current;
    if (!value || phaseNow.current !== "recording") return;
    const epoch = generation.current;
    setPhase("stopping");
    try {
      await importRecording(await value.stop(), epoch, false);
    } catch (reason) {
      if (own(epoch)) {
        setError(assistantErrorKey(reason));
        setPhase("idle");
      }
    }
  };
  const cancelRecording = () => {
    generation.current++;
    permission.current?.abort();
    permission.current = null;
    recording.current?.cancel();
    recording.current = null;
    setPhase("idle");
    setSeconds(0);
  };
  const transcribe = async () => {
    if (phaseNow.current !== "idle" || !inputNow.current || !currentModel)
      return;
    const epoch = generation.current;
    pendingModels.current = models;
    const run = {
      runId: crypto.randomUUID(),
      operationId: crypto.randomUUID(),
    };
    request.current = run;
    stopSent.current = false;
    setStopping(false);
    setPhase("transcribing");
    setError(null);
    setResult(null);
    setText("");
    try {
      const value = await assistantApi.transcribe({
        ...target,
        ...run,
        modelId,
        inputId: inputNow.current.inputId,
        ...(keepOriginal ? { keepOriginal: true } : {}),
        ...(language === "auto"
          ? {}
          : { language: language as "zh" | "en" | "ja" }),
      });
      if (!own(epoch)) return;
      inputNow.current = null;
      setInput(null);
      selectResult(value);
    } catch (reason) {
      if (own(epoch)) setError(assistantErrorKey(reason));
    } finally {
      if (own(epoch)) {
        request.current = null;
        pendingModels.current = null;
        setPhase("idle");
      }
    }
  };
  const stopTranscription = () => {
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
  const price = currentModel?.audio?.pricing;
  const rawRate = price?.inputAudioPerSecond;
  const rate = rawRate == null || rawRate === "" ? NaN : Number(rawRate);
  const estimate =
    input && price?.unit === "seconds" && Number.isFinite(rate) && rate >= 0
      ? (rate * input.durationMs) / 1000
      : null;
  const number = (value: number) =>
    value.toLocaleString(i18n.language, { maximumFractionDigits: 2 });
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
      <DialogContent className="max-w-[520px]" zIndex="nested">
        <DialogHeader>
          <DialogTitle>{t("assistant.audio.title")}</DialogTitle>
        </DialogHeader>
        <div className="flex min-h-0 flex-col gap-3 overflow-y-auto px-[18px] py-4">
          <DialogDescription className="text-note text-ink-muted">
            {t("assistant.audio.hint")}
          </DialogDescription>
          <div className="flex flex-wrap items-center gap-2">
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void importFile()}
            >
              <Upload aria-hidden className="mr-1.5 h-3.5 w-3.5" />
              {t("assistant.audio.import")}
            </Button>
            <Button
              variant="secondary"
              disabled={busy}
              onClick={() => void record()}
            >
              <Mic aria-hidden className="mr-1.5 h-3.5 w-3.5" />
              {t("assistant.audio.record")}
            </Button>
            {phase === "recording" && (
              <Button
                variant="secondary"
                onClick={() => void finishRecording()}
              >
                <Square aria-hidden className="mr-1.5 h-3.5 w-3.5" />
                {t("assistant.audio.finishRecording")}
              </Button>
            )}
            {(phase === "recording" || phase === "starting") && (
              <Button variant="ghost" onClick={cancelRecording}>
                {t("assistant.audio.discardRecording")}
              </Button>
            )}
          </div>
          {phase === "starting" && (
            <p role="status" className="text-note text-ink-muted">
              {t("assistant.audio.waiting")}
            </p>
          )}
          {phase === "recording" && (
            <p role="status" className="text-note tabular-nums text-brand-ink">
              {t("assistant.audio.recording", {
                seconds: number(Math.floor(seconds)),
              })}
            </p>
          )}
          {limited && (
            <p role="status" className="text-note text-ink-muted">
              {t("assistant.audio.limit")}
            </p>
          )}
          {input && (
            <div className="rounded-ctl border border-line px-3 py-2">
              <p className="truncate text-ctl text-ink" title={input.name}>
                {input.name}
              </p>
              <p className="text-note text-ink-muted">
                {t("assistant.audio.inputInfo", {
                  seconds: number(input.durationMs / 1000),
                  size: `${number(input.sizeBytes / 1024)} KiB`,
                })}
              </p>
            </div>
          )}
          <div className="flex flex-wrap items-end gap-3">
            <div className="flex min-w-0 flex-col gap-1.5">
              <label
                htmlFor={`${ids}-model`}
                className="text-note text-ink-muted"
              >
                {t("assistant.audio.model")}
              </label>
              <MenuSelect
                id={`${ids}-model`}
                aria-label={t("assistant.audio.model")}
                value={modelId}
                onChange={setModelId}
                disabled={busy}
                options={options.map((model) => ({
                  value: model.id,
                  label: model.name,
                }))}
                searchable
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
                onChange={setLanguage}
                disabled={busy}
                options={["auto", "zh", "en", "ja"].map((value) => ({
                  value,
                  label: t(`assistant.audio.${value}`),
                }))}
              />
            </div>
          </div>
          <p className="text-note text-ink-muted">
            {t("assistant.audio.languageHint")}
          </p>
          <label className="flex items-center gap-2 text-note text-ink">
            <Switch
              checked={keepOriginal}
              onCheckedChange={setKeepOriginal}
              disabled={busy}
              aria-label={t("assistant.audio.keepOriginal")}
            />
            {t("assistant.audio.keepOriginal")}
          </label>
          <p className="text-note text-ink-muted">
            {t("assistant.audio.keepOriginalHint")}
          </p>
          {input && (
            <p className="text-note text-ink-muted">
              {estimate === null
                ? t("assistant.audio.unknownPrice")
                : t("assistant.audio.priceHint", {
                    amount: usd.format(estimate),
                  })}
            </p>
          )}
          {phase === "transcribing" && (
            <div
              role="status"
              className="flex flex-col items-start gap-2 text-note text-ink-muted"
            >
              <span className="flex items-center gap-2">
                <Loader2 aria-hidden className="h-3.5 w-3.5 animate-spin" />
                {t("assistant.audio.transcribing")}
              </span>
              <p>{t("assistant.audio.submittedHint")}</p>
              <Button
                variant="secondary"
                disabled={stopping}
                onClick={stopTranscription}
              >
                {t(
                  stopping
                    ? "assistant.audio.stopping"
                    : "assistant.audio.stop",
                )}
              </Button>
            </div>
          )}
          {result && (
            <div className="flex flex-col gap-2">
              {result.status === "completed" ? (
                <>
                  <label htmlFor={`${ids}-draft`} className="text-ctl text-ink">
                    {t("assistant.audio.draft")}
                  </label>
                  <Textarea
                    id={`${ids}-draft`}
                    value={text}
                    onChange={(event) => setText(event.target.value)}
                    rows={5}
                    className="min-h-[100px] resize-y"
                  />
                  {!result.audio.text && (
                    <p className="text-note text-ink-muted">
                      {t("assistant.audio.empty")}
                    </p>
                  )}
                </>
              ) : (
                <p role="alert" className="text-note text-danger">
                  {t(
                    assistantErrorKey(
                      result.audio.errorCode ??
                        (result.status === "not_sent"
                          ? "assistant_cancelled"
                          : "assistant_audio_outcome_unknown"),
                    ),
                  )}
                </p>
              )}
              {result.inputIds?.[0] && result.audio.input && (
                <AssistantAudioPlayer
                  key={`${result.operationId}:${result.inputIds[0]}`}
                  target={target}
                  attachmentId={result.inputIds[0]}
                  mimeType={result.audio.input.mimeType}
                  durationMs={result.audio.input.durationMs}
                />
              )}
              {result.audio.originalError && (
                <p role="alert" className="text-note text-danger">
                  {t("assistant.audio.originalFailed")}
                </p>
              )}
              <p className="text-note text-ink-muted">
                {result.cost === null
                  ? t("assistant.audio.unknownCost")
                  : t("assistant.audio.cost", {
                      amount: usd.format(Number(result.cost)),
                    })}
              </p>
            </div>
          )}
          {history.length > 0 && (
            <div className="flex flex-col gap-1">
              <p className="text-note font-medium text-ink-muted">
                {t("assistant.audio.history")}
              </p>
              <div className="max-h-[110px] overflow-y-auto">
                {[...history].reverse().map((item, index) => (
                  <Button
                    key={item.operationId}
                    variant="ghost"
                    disabled={busy}
                    className="h-auto w-full justify-start truncate py-1.5 text-note"
                    onClick={() => selectResult(item)}
                  >
                    {t("assistant.audio.historyItem", {
                      number: history.length - index,
                    })}
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
          {result?.status === "completed" ? (
            <Button
              disabled={!text.trim() || busy}
              onClick={() => {
                if (onInsert(text)) close();
                else setError("assistant.audio.selectionChanged");
              }}
            >
              {t("assistant.audio.insert")}
            </Button>
          ) : (
            <Button
              disabled={!input || !currentModel || busy}
              onClick={() => void transcribe()}
            >
              {t("assistant.audio.transcribe")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
