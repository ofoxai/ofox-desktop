import { useCallback, useEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import {
  assistantApi,
  assistantErrorKey,
  assistantMediaUrl,
  type AssistantBootstrap,
  type AssistantModel,
  type AssistantPasteTarget,
} from "@/lib/api/assistant";

/** The speech model, voice and format a read-aloud uses without asking. */
export function readAloudChoice(bootstrap: AssistantBootstrap | null): {
  model: AssistantModel;
  voice: string;
  format: string;
} | null {
  const models = (bootstrap?.audioModels ?? []).filter(
    (model) => model.kind === "speech" && model.audio,
  );
  const model =
    models.find((item) => item.id === bootstrap?.rememberedModels?.speech) ??
    models[0];
  const audio = model?.audio;
  if (!model || !audio) return null;
  const saved = bootstrap?.speechPreferences?.[model.id];
  const voicePattern =
    audio.voiceKind === "id" ? /^[A-Za-z0-9]{20}$/ : /^[A-Za-z]{1,64}$/;
  const voice =
    saved?.voice && voicePattern.test(saved.voice)
      ? saved.voice
      : (audio.suggestedVoices[0] ?? "");
  const format =
    saved?.format &&
    saved.format !== "pcm" &&
    audio.responseFormats.includes(saved.format)
      ? saved.format
      : audio.defaultFormat;
  if (!voice || format === "pcm") return null;
  return { model, voice, format };
}

/**
 * A reply read aloud in one click, with the remembered speech model and
 * voice: no dialog. A second click on the same reply stops it.
 */
export function useReadAloud({
  bootstrap,
  prepare,
}: {
  bootstrap: AssistantBootstrap | null;
  prepare: () => Promise<
    (AssistantPasteTarget & { selectionKey: number }) | null
  >;
}) {
  const { t } = useTranslation();
  const [speaking, setSpeaking] = useState<string | null>(null);
  const audio = useRef<HTMLAudioElement | null>(null);
  const stop = useCallback(() => {
    audio.current?.pause();
    audio.current = null;
    setSpeaking(null);
  }, []);
  useEffect(() => stop, [stop]);
  const speak = useCallback(
    async (messageId: string, text: string) => {
      if (speaking === messageId) {
        stop();
        return;
      }
      stop();
      const choice = readAloudChoice(bootstrap);
      if (!choice) return;
      const target = await prepare();
      if (!target) return;
      setSpeaking(messageId);
      try {
        const result = await assistantApi.synthesize({
          sessionId: target.sessionId,
          expectedOwner: target.expectedOwner,
          expectedGeneration: target.expectedGeneration,
          runId: crypto.randomUUID(),
          operationId: crypto.randomUUID(),
          modelId: choice.model.id,
          input: Array.from(text).slice(0, 4000).join(""),
          voice: choice.voice,
          responseFormat: choice.format,
          sourceMessageId: messageId,
        });
        const output = result.outputs?.[0];
        if (result.status !== "completed" || !output) {
          setSpeaking(null);
          toast.error(t("assistant.generation.readAloudFailed"));
          return;
        }
        const player = new Audio(
          assistantMediaUrl(
            target.sessionId,
            output,
            target.expectedOwner,
            target.expectedGeneration,
          ),
        );
        audio.current = player;
        player.onended = () => {
          if (audio.current === player) stop();
        };
        await player.play();
      } catch (error) {
        setSpeaking(null);
        toast.error(t(assistantErrorKey(error)));
      }
    },
    [bootstrap, prepare, speaking, stop, t],
  );
  return { speak, speaking, available: Boolean(readAloudChoice(bootstrap)) };
}
