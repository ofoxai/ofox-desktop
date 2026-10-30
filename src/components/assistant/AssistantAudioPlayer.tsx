import { useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { Download, Loader2, Pause, Play, Square } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useInteractionActive } from "@/components/ui/interaction-scope";
import {
  assistantApi,
  assistantErrorKey,
  assistantMediaUrl,
  type AssistantPasteTarget,
} from "@/lib/api/assistant";

export interface AssistantAudioPlayerProps {
  target: AssistantPasteTarget;
  attachmentId: string;
  mimeType: string;
  durationMs?: number;
  disabled?: boolean;
}
const seconds = (value: number) =>
  Number.isFinite(value) && value >= 0 && value <= 3600 ? value : null;
const stamp = (value: number | null) =>
  value == null
    ? "--:--"
    : `${Math.floor(value / 60)}:${Math.floor(value % 60)
        .toString()
        .padStart(2, "0")}`;
function release(audio: HTMLAudioElement | null) {
  if (!audio?.getAttribute("src")) return;
  audio.pause();
  try {
    audio.currentTime = 0;
  } catch {
    /* An uninitialized decoder may refuse seeking. */
  }
  audio.removeAttribute("src");
  audio.load();
}
/** Playback owns only an already generated private file, never a model request. */
export function AssistantAudioPlayer({
  target,
  attachmentId,
  mimeType,
  durationMs,
  disabled = false,
}: AssistantAudioPlayerProps) {
  const { t } = useTranslation();
  const active = useInteractionActive();
  const audio = useRef<HTMLAudioElement>(null);
  const lifetime = useRef(0);
  const attempt = useRef(0);
  const savingNow = useRef(false);
  const [available, setAvailable] = useState(false);
  const [playing, setPlaying] = useState(false);
  const [pending, setPending] = useState(false);
  const [saving, setSaving] = useState(false);
  const [saved, setSaved] = useState(false);
  const [elapsed, setElapsed] = useState(0);
  const fallback = durationMs == null ? null : seconds(durationMs / 1000);
  const [duration, setDuration] = useState(fallback);
  const [playError, setPlayError] = useState<string | null>(null);
  const [saveError, setSaveError] = useState<string | null>(null);
  const url = assistantMediaUrl(
    target.sessionId,
    attachmentId,
    target.expectedOwner,
    target.expectedGeneration,
  );
  const codec =
    mimeType === "audio/ogg" ? 'audio/ogg; codecs="opus"' : mimeType;
  useEffect(() => {
    const element = audio.current;
    lifetime.current++;
    attempt.current++;
    savingNow.current = false;
    setPlaying(false);
    setPending(false);
    setSaving(false);
    setSaved(false);
    setElapsed(0);
    setDuration(fallback);
    setPlayError(null);
    setSaveError(null);
    setAvailable(Boolean(element?.canPlayType(codec)));
    if (!active || disabled) release(element);
    return () => {
      lifetime.current++;
      attempt.current++;
      release(element);
    };
  }, [url, codec, active, disabled, fallback]);
  const follows = (element: HTMLAudioElement) =>
    active && !disabled && element.getAttribute("src") === url;
  const play = async () => {
    const element = audio.current;
    if (!element || !active || disabled || pending || !available) return;
    const epoch = lifetime.current;
    const ticket = ++attempt.current;
    setPending(true);
    setPlayError(null);
    setSaved(false);
    if (element.getAttribute("src") !== url) element.setAttribute("src", url);
    if (element.ended) element.currentTime = 0;
    try {
      await element.play();
      if (epoch === lifetime.current && ticket === attempt.current)
        setPlaying(true);
    } catch (reason) {
      if (epoch === lifetime.current && ticket === attempt.current) {
        const name =
          reason && typeof reason === "object" && "name" in reason
            ? reason.name
            : "";
        if (name === "NotSupportedError") setAvailable(false);
        setPlayError(
          name === "NotAllowedError"
            ? "assistant.audioPlayer.blocked"
            : "assistant.audioPlayer.failed",
        );
        setPlaying(false);
        release(element);
      }
    } finally {
      if (epoch === lifetime.current && ticket === attempt.current)
        setPending(false);
    }
  };
  const pause = () => {
    attempt.current++;
    audio.current?.pause();
    setPlaying(false);
    setPending(false);
  };
  const stop = () => {
    attempt.current++;
    release(audio.current);
    setPlaying(false);
    setPending(false);
    setElapsed(0);
  };
  const save = async () => {
    if (!active || disabled || savingNow.current) return;
    const epoch = lifetime.current;
    savingNow.current = true;
    setSaving(true);
    setSaveError(null);
    setSaved(false);
    try {
      const value = await assistantApi.saveMedia({
        sessionId: target.sessionId,
        expectedOwner: target.expectedOwner,
        expectedGeneration: target.expectedGeneration,
        attachmentId,
      });
      if (epoch === lifetime.current) setSaved(value);
    } catch (reason) {
      if (epoch === lifetime.current) setSaveError(assistantErrorKey(reason));
    } finally {
      if (epoch === lifetime.current) {
        savingNow.current = false;
        setSaving(false);
      }
    }
  };
  return (
    <div
      role="group"
      aria-label={t("assistant.audioPlayer.title")}
      className="flex flex-col gap-2 rounded-list border border-line p-3"
    >
      <audio
        ref={audio}
        preload="none"
        className="hidden"
        onLoadedMetadata={(event) => {
          if (follows(event.currentTarget))
            setDuration(seconds(event.currentTarget.duration) ?? fallback);
        }}
        onTimeUpdate={(event) => {
          if (follows(event.currentTarget))
            setElapsed(seconds(event.currentTarget.currentTime) ?? 0);
        }}
        onPlaying={(event) => {
          if (follows(event.currentTarget)) {
            setPlaying(true);
            setPending(false);
          }
        }}
        onPause={(event) => {
          if (follows(event.currentTarget)) {
            attempt.current++;
            setPlaying(false);
            setPending(false);
          }
        }}
        onEnded={(event) => {
          if (follows(event.currentTarget)) {
            attempt.current++;
            setPlaying(false);
            setPending(false);
            setElapsed(seconds(event.currentTarget.duration) ?? duration ?? 0);
          }
        }}
        onError={(event) => {
          if (follows(event.currentTarget)) {
            attempt.current++;
            setPlaying(false);
            setPending(false);
            setPlayError("assistant.audioPlayer.failed");
            release(event.currentTarget);
          }
        }}
      />
      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="secondary"
          disabled={!active || disabled || !available || pending}
          onClick={() => (playing ? pause() : void play())}
        >
          {pending ? (
            <Loader2 aria-hidden className="mr-1.5 h-3.5 w-3.5 animate-spin" />
          ) : playing ? (
            <Pause aria-hidden className="mr-1.5 h-3.5 w-3.5" />
          ) : (
            <Play aria-hidden className="mr-1.5 h-3.5 w-3.5" />
          )}
          {t(
            playing
              ? "assistant.audioPlayer.pause"
              : "assistant.audioPlayer.play",
          )}
        </Button>
        <Button variant="ghost" disabled={!active || disabled} onClick={stop}>
          <Square aria-hidden className="mr-1.5 h-3.5 w-3.5" />
          {t("assistant.audioPlayer.stop")}
        </Button>
        <Button
          variant="ghost"
          disabled={!active || disabled || saving}
          onClick={() => void save()}
        >
          <Download aria-hidden className="mr-1.5 h-3.5 w-3.5" />
          {t("assistant.audioPlayer.save")}
        </Button>
        <span
          role="timer"
          aria-label={t("assistant.audioPlayer.position")}
          className="text-note tabular-nums text-ink-muted"
        >
          {stamp(Math.min(elapsed, duration ?? 3600))} / {stamp(duration)}
        </span>
      </div>
      {!available && (
        <p className="text-note text-ink-muted">
          {t("assistant.audioPlayer.unsupported")}
        </p>
      )}
      {playError && (
        <p role="alert" className="text-note text-danger">
          {t(playError)}
        </p>
      )}
      {saveError && (
        <p role="alert" className="text-note text-danger">
          {t(saveError)}
        </p>
      )}
      {saved && (
        <p role="status" className="text-note text-ink-muted">
          {t("assistant.audioPlayer.saved")}
        </p>
      )}
    </div>
  );
}
