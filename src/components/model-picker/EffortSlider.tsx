import { useEffect, useId, useRef, useState } from "react";
import type { KeyboardEvent, PointerEvent } from "react";
import { useTranslation } from "react-i18next";
import { toast } from "sonner";
import { useModelCatalog, useSaveEffort } from "@/hooks/useModelSelection";
import type { ToolModelSnapshot } from "@/lib/api/modelSelection";
import { cn } from "@/lib/utils";
import { effortName, effortStops } from "./effortLevels";

interface EffortSliderProps {
  toolId: string;
  snapshot: ToolModelSnapshot;
  disabled?: boolean;
}

/** The rail's inset: the knob's half width, so the end stops sit under its centre. */
const INSET = 8;

/**
 * magpie's 推理强度: the tool's default and its levels as stops on a rail,
 * the current one named at the right. It shows only for a tool Ofox can set
 * the effort on, and only while its model is one the catalog marks as
 * reasoning. A pick shows at once and saves in the order made; a failed save
 * puts the slider back and says why.
 */
export function EffortSlider({
  toolId,
  snapshot,
  disabled = false,
}: EffortSliderProps) {
  const { t } = useTranslation();
  const labelId = useId();
  const effort = snapshot.effort;
  const model =
    snapshot.managedSelection.kind === "single"
      ? snapshot.managedSelection.model
      : "";
  const catalog = useModelCatalog(toolId, !!effort && !!model);
  const save = useSaveEffort();
  const [picked, setPicked] = useState<string | null | undefined>(undefined);
  const [dragging, setDragging] = useState<number | null>(null);
  const queue = useRef(Promise.resolve());
  const revision = useRef(snapshot.revision);
  const track = useRef<HTMLDivElement>(null);

  useEffect(() => {
    revision.current = snapshot.revision;
  }, [snapshot.revision]);
  // Once the snapshot says what was picked, it speaks for itself again.
  useEffect(() => {
    if (picked !== undefined && effort?.value === picked) setPicked(undefined);
  }, [effort?.value, picked]);

  const reasons = catalog.models
    .find((entry) => entry.id === model)
    ?.supportedParameters?.includes("reasoning");
  if (!effort || !model || !reasons) return null;

  const value = picked === undefined ? effort.value : picked;
  const stops = effortStops(effort, value);
  const last = stops.length - 1;
  const current = stops.indexOf(value);
  const shown = dragging ?? current;
  const share = last > 0 ? shown / last : 0;
  const at = (fraction: number) =>
    `calc(${INSET}px + (100% - ${INSET * 2}px) * ${fraction})`;
  const name = (stop: string | null) => effortName(stop, effort, t);

  const pick = (index: number) => {
    const next = stops[Math.min(last, Math.max(0, index))];
    if (disabled || next === value) return;
    setPicked(next);
    queue.current = queue.current.then(async () => {
      try {
        const fresh = await save.mutateAsync({
          tool: toolId,
          effort: next,
          expectedRevision: revision.current,
        });
        revision.current = fresh.revision;
      } catch (error) {
        setPicked(undefined);
        toast.error(t("effort.saveFailed", { error: String(error) }));
      }
    });
  };
  const stopAt = (clientX: number) => {
    const rect = track.current?.getBoundingClientRect();
    if (!rect || rect.width <= INSET * 2) return current;
    const fraction = (clientX - rect.left - INSET) / (rect.width - INSET * 2);
    return Math.min(last, Math.max(0, Math.round(fraction * last)));
  };
  const onKeyDown = (event: KeyboardEvent) => {
    const next =
      event.key === "ArrowLeft" || event.key === "ArrowDown"
        ? current - 1
        : event.key === "ArrowRight" || event.key === "ArrowUp"
          ? current + 1
          : event.key === "Home"
            ? 0
            : event.key === "End"
              ? last
              : null;
    if (next === null) return;
    event.preventDefault();
    pick(next);
  };
  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (disabled) return;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    setDragging(stopAt(event.clientX));
  };
  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    if (dragging !== null) setDragging(stopAt(event.clientX));
  };
  const onPointerUp = (event: PointerEvent<HTMLDivElement>) => {
    if (dragging === null) return;
    const index = stopAt(event.clientX);
    setDragging(null);
    pick(index);
  };

  return (
    <div className={cn("space-y-1", disabled && "opacity-55")}>
      <div className="flex items-baseline gap-2">
        <span id={labelId} className="text-note text-ink-muted">
          {t("effort.label")}
        </span>
        <b className="ml-auto text-ctl font-semibold text-ink">
          {name(stops[shown] ?? value)}
        </b>
      </div>
      <div
        ref={track}
        role="slider"
        tabIndex={disabled ? -1 : 0}
        aria-labelledby={labelId}
        aria-valuemin={0}
        aria-valuemax={last}
        aria-valuenow={shown}
        aria-valuetext={name(stops[shown] ?? value)}
        aria-disabled={disabled || undefined}
        onKeyDown={onKeyDown}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => setDragging(null)}
        className="group relative h-5 cursor-pointer touch-none select-none rounded-full focus-visible:outline-none aria-disabled:cursor-default"
      >
        <span
          aria-hidden
          className="absolute top-1/2 h-1 -translate-y-1/2 rounded-full bg-line"
          style={{ left: INSET, right: INSET }}
        />
        <span
          aria-hidden
          className="absolute top-1/2 h-1 -translate-y-1/2 rounded-full bg-ink transition-[width] duration-200 ease-soft motion-reduce:transition-none"
          style={{
            left: INSET,
            width: `calc((100% - ${INSET * 2}px) * ${share})`,
          }}
        />
        {stops.map((stop, index) => (
          <span
            key={stop ?? "default"}
            aria-hidden
            className={cn(
              "absolute top-1/2 h-1 w-1 -translate-x-1/2 -translate-y-1/2 rounded-full",
              index < shown ? "bg-card" : "bg-ink-faint",
            )}
            style={{ left: at(last > 0 ? index / last : 0) }}
          />
        ))}
        <span
          aria-hidden
          className="absolute top-1/2 h-4 w-4 -translate-x-1/2 -translate-y-1/2 rounded-full bg-white shadow-[0_1px_3px_rgba(0,0,0,.25),0_0_0_.5px_rgba(0,0,0,.08)] transition-[left] duration-200 ease-soft group-focus-visible:ring-2 group-focus-visible:ring-brand/50 motion-reduce:transition-none"
          style={{ left: at(share) }}
        />
      </div>
      <div className="flex justify-between text-micro text-ink-muted">
        <span>{name(stops[0])}</span>
        <span>{name(stops[last])}</span>
      </div>
    </div>
  );
}
