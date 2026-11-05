import { useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AudioLines,
  ChevronDown,
  Clapperboard,
  ImageIcon,
  Paperclip,
  Plus,
  X,
} from "lucide-react";
import type {
  AssistantGenerationKind,
  AssistantMediaDefaults,
  AssistantModel,
} from "@/lib/api/assistant";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import {
  MediaDefaultsFields,
  generationModel,
  generationModels,
  type MediaFieldsValue,
} from "./MediaDefaultsFields";

export const GENERATION_KINDS: AssistantGenerationKind[] = [
  "image",
  "video",
  "speech",
];

const ICONS: Record<AssistantGenerationKind, typeof ImageIcon> = {
  image: ImageIcon,
  video: Clapperboard,
  speech: AudioLines,
};

/** The parameters that are not the default, for the chip's tooltip. */
function summary(
  kind: AssistantGenerationKind,
  value: MediaFieldsValue | undefined,
  t: (key: string, options?: Record<string, unknown>) => string,
): string {
  if (!value) return "";
  if (kind === "image") {
    const image = value as AssistantMediaDefaults["image"];
    return [
      image?.size,
      image?.quality,
      image?.n && image.n > 1 ? `×${image.n}` : "",
    ]
      .filter(Boolean)
      .join(" · ");
  }
  if (kind === "video") {
    const video = value as AssistantMediaDefaults["video"];
    return [
      video?.duration
        ? t("assistant.video.seconds", { count: video.duration })
        : "",
      video?.resolution,
      video?.aspectRatio,
      video?.generateAudio === false ? t("assistant.generation.silent") : "",
    ]
      .filter(Boolean)
      .join(" · ");
  }
  const speech = value as AssistantMediaDefaults["speech"];
  return [speech?.voice, speech?.format].filter(Boolean).join(" · ");
}

/**
 * The composer's "+": add files, or ask for a generation, which becomes a
 * chip beside it. A chip names only what it makes; its model and parameters
 * are in its tooltip and the panel it opens. × removes it.
 */
export function ComposerPlus({
  models,
  media,
  onMedia,
  canAttach,
  onAttach,
  disabled,
}: {
  models: ReadonlyArray<AssistantModel>;
  media: AssistantMediaDefaults;
  onMedia: (media: AssistantMediaDefaults) => void;
  canAttach: boolean;
  onAttach: () => void;
  disabled: boolean;
}) {
  const { t } = useTranslation();
  const [open, setOpen] = useState<AssistantGenerationKind | null>(null);
  // An entry that moves focus on (the file dialog, a chip's parameters)
  // keeps the menu from handing it back to "+".
  const focusMoves = useRef(false);
  const hints = media.hints ?? [];
  const setHints = (next: AssistantGenerationKind[]) =>
    onMedia({ ...media, hints: next });
  const add = (kind: AssistantGenerationKind) => {
    if (!hints.includes(kind)) setHints([...hints, kind]);
    setOpen(kind);
  };
  return (
    <div className="flex min-w-0 flex-wrap items-center gap-1.5">
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="icon"
            aria-label={t("assistant.generation.plus")}
            title={t("assistant.generation.plus")}
            disabled={disabled}
          >
            <Plus aria-hidden className="h-4 w-4" />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent
          align="start"
          className="w-52"
          onCloseAutoFocus={(event) => {
            if (focusMoves.current) event.preventDefault();
            focusMoves.current = false;
          }}
        >
          <DropdownMenuItem
            disabled={!canAttach}
            onSelect={() => {
              focusMoves.current = true;
              onAttach();
            }}
          >
            <Paperclip aria-hidden className="h-3.5 w-3.5" />
            {t("assistant.generation.attach")}
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          {GENERATION_KINDS.map((kind) => {
            const Icon = ICONS[kind];
            return (
              <DropdownMenuItem
                key={kind}
                disabled={generationModels(models, kind).length === 0}
                onSelect={() => {
                  focusMoves.current = true;
                  add(kind);
                }}
              >
                <Icon aria-hidden className="h-3.5 w-3.5" />
                {t(`assistant.generation.kinds.${kind}`)}
              </DropdownMenuItem>
            );
          })}
        </DropdownMenuContent>
      </DropdownMenu>
      {hints.map((kind) => {
        const Icon = ICONS[kind];
        const value = media[kind];
        const model = generationModel(models, kind, value?.model);
        const name = t(`assistant.generation.kinds.${kind}`);
        const detail = [model?.name, summary(kind, value, t)]
          .filter(Boolean)
          .join(" · ");
        return (
          <span
            key={kind}
            className="inline-flex shrink-0 items-center rounded-ctl bg-brand-soft text-brand-ink"
          >
            <Popover
              open={open === kind}
              onOpenChange={(next) => setOpen(next ? kind : null)}
            >
              <PopoverTrigger asChild>
                <Button
                  variant="ghost"
                  disabled={disabled}
                  aria-label={t("assistant.generation.chipLabel", { name })}
                  title={detail || undefined}
                  className="h-7 gap-1 px-2 text-brand-ink hover:bg-transparent hover:text-ink"
                >
                  <Icon aria-hidden className="h-3.5 w-3.5 shrink-0" />
                  {name}
                  <ChevronDown aria-hidden className="h-3 w-3 opacity-60" />
                </Button>
              </PopoverTrigger>
              <PopoverContent align="start" className="w-[420px] max-w-[90vw]">
                <p className="mb-2 text-note text-ink-muted">
                  {t("assistant.generation.defaultsNote")}
                </p>
                <MediaDefaultsFields
                  kind={kind}
                  models={models}
                  value={value ?? {}}
                  disabled={disabled}
                  onChange={(next) => onMedia({ ...media, [kind]: next })}
                />
              </PopoverContent>
            </Popover>
            <button
              type="button"
              aria-label={t("assistant.generation.remove", { name })}
              disabled={disabled}
              className="mr-1 rounded-ctl p-0.5 hover:text-ink disabled:opacity-50"
              onClick={() => setHints(hints.filter((item) => item !== kind))}
            >
              <X aria-hidden className="h-3 w-3" />
            </button>
          </span>
        );
      })}
    </div>
  );
}
