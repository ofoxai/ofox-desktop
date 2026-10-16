import { useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { SegmentedControl } from "@/components/ui/segmented";
import { MenuSelect } from "@/components/ui/menu-select";
import { fmtTokens } from "@/lib/format";
import { formatUsageEstimate } from "@/lib/usageEstimate";
import { formatActivity, heatColor, heatColors } from "@/lib/usageInsights";
import type { SessionAnalytics } from "@/types/sessionAnalytics";

type CalendarMetric = "tokens" | "sessions" | "messages" | "active" | "cost";
export function UsageTimeInsights({ data }: { data: SessionAnalytics }) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<"hourly" | "calendar">("hourly");
  const [metric, setMetric] = useState<CalendarMetric>("tokens");
  const years = useMemo(
    () => [...new Set(data.series.map((day) => day.date.slice(0, 4)))],
    [data.series],
  );
  const [chosenYear, setYear] = useState("");
  const year = years.includes(chosenYear) ? chosenYear : years.at(-1);
  const days = data.series.filter((day) => day.date.startsWith(year ?? ""));
  const weekdays = t("usageExperience.weekdays").split(",");
  const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const hourly = data.activityByHour;
  const hourlyAvailable = !!hourly && hourly.coverage.state !== "unavailable";
  const cells = Array.from({ length: 168 }, (_, index) => {
    const weekday = Math.floor(index / 24);
    const hour = index % 24;
    const amount = hourlyAvailable
      ? (hourly.cells.find(
          (cell) => cell.weekday === weekday && cell.hour === hour,
        )?.activeMs ?? 0)
      : null;
    return {
      id: `${weekday}:${hour}`,
      amount,
      label: t("usageExperience.hourCell", {
        day: weekdays[weekday],
        hour: `${String(hour).padStart(2, "0")}:00`,
        value: formatActivity(amount, t),
      }),
    };
  });
  const calendar = days.map((day) => {
    const amount =
      metric === "tokens"
        ? day.totalTokens
        : metric === "sessions"
          ? day.sessionCount
          : metric === "messages"
            ? (day.messages ?? day.knownMessages ?? null)
            : metric === "active"
              ? (day.activeMs ?? day.knownActiveMs ?? null)
              : day.estimate.knownCostUsd == null
                ? null
                : Number(day.estimate.knownCostUsd);
    const value =
      metric === "active"
        ? formatActivity(amount, t)
        : metric === "cost"
          ? formatUsageEstimate(day.estimate)
          : amount == null
            ? "—"
            : fmtTokens(amount);
    return {
      id: day.date,
      amount,
      label: t("usageExperience.calendarCell", { date: day.date, value }),
    };
  });
  const shown = mode === "hourly" ? cells : calendar;
  const max = Math.max(0, ...shown.map((cell) => cell.amount ?? 0));
  const busiest = shown.reduce<{ amount: number; label: string } | null>(
    (best, cell) =>
      cell.amount != null && cell.amount > (best?.amount ?? 0)
        ? { amount: cell.amount, label: cell.label }
        : best,
    null,
  );
  const [focused, setFocused] = useState(0);
  const [description, setDescription] = useState("");
  const refs = useRef<(HTMLButtonElement | null)[]>([]);
  const renderCell = (cell: (typeof shown)[number], index: number) => (
    <button
      key={cell.id}
      type="button"
      ref={(node) => {
        refs.current[index] = node;
      }}
      tabIndex={index === Math.min(focused, shown.length - 1) ? 0 : -1}
      aria-label={cell.label}
      title={cell.label}
      className={`h-5 min-w-0 rounded-[3px] outline-none focus-visible:ring-2 focus-visible:ring-brand-ink ${heatColor(cell.amount, max)}`}
      onFocus={() => {
        setFocused(index);
        setDescription(cell.label);
      }}
      onClick={() => {
        setFocused(index);
        setDescription(cell.label);
      }}
      onKeyDown={(event) => {
        const delta =
          mode === "hourly"
            ? { ArrowRight: 1, ArrowLeft: -1, ArrowDown: 24, ArrowUp: -24 }
            : { ArrowRight: 7, ArrowLeft: -7, ArrowDown: 1, ArrowUp: -1 };
        let next: number | undefined;
        if (event.key === "Home") next = 0;
        else if (event.key === "End") next = shown.length - 1;
        else if (event.key in delta)
          next = Math.max(
            0,
            Math.min(
              shown.length - 1,
              index + delta[event.key as keyof typeof delta],
            ),
          );
        if (next != null) {
          event.preventDefault();
          refs.current[next]?.focus();
        }
      }}
    />
  );
  return (
    <section
      className="min-w-0 rounded-list border border-line bg-card p-4"
      aria-label={t("usageExperience.timeInsights")}
    >
      <div className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h2 className="text-sub font-medium text-ink-muted">
          {t("usageExperience.timeInsights")}
        </h2>
        <SegmentedControl
          aria-label={t("usageExperience.timeInsights")}
          value={mode}
          onChange={(value) => {
            setMode(value);
            setFocused(0);
            setDescription("");
          }}
          options={[
            { value: "hourly", label: t("usageExperience.hourly") },
            { value: "calendar", label: t("usageExperience.calendar") },
          ]}
        />
      </div>
      {mode === "hourly" ? (
        <>
          <p className="mb-2 text-right text-note text-ink-faint">{timezone}</p>
          {hourlyAvailable ? (
            <div className="grid grid-cols-[28px_minmax(0,1fr)] gap-2">
              <div
                aria-hidden
                className="grid grid-rows-7 gap-1 text-note text-ink-muted"
              >
                {weekdays.map((day) => (
                  <span key={day} className="flex h-5 items-center">
                    {day}
                  </span>
                ))}
              </div>
              <div>
                <div className="grid grid-cols-[repeat(24,minmax(0,1fr))] gap-1">
                  {cells.map(renderCell)}
                </div>
                <div
                  aria-hidden
                  className="mt-1 grid grid-cols-4 text-note text-ink-faint"
                >
                  {[0, 6, 12, 18].map((hour) => (
                    <span key={hour}>{hour}</span>
                  ))}
                </div>
              </div>
            </div>
          ) : (
            <p className="flex min-h-[190px] items-center justify-center text-sub text-ink-muted">
              {t("usageExperience.noActivity")}
            </p>
          )}
        </>
      ) : (
        <>
          <div className="mb-3 flex items-center justify-end gap-2">
            {years.length > 1 && (
              <MenuSelect
                aria-label={t("usageExperience.calendar")}
                value={year ?? ""}
                onChange={setYear}
                options={years.map((value) => ({ value, label: value }))}
              />
            )}
            <MenuSelect<CalendarMetric>
              aria-label={t("usageExperience.calendarMetric")}
              value={metric}
              onChange={(value) => {
                setMetric(value);
                setDescription("");
              }}
              options={(
                ["tokens", "sessions", "messages", "active", "cost"] as const
              ).map((value) => ({
                value,
                label: t(`usageWorkbench.${value}`),
              }))}
            />
          </div>
          <div className="overflow-x-auto pb-1">
            <div
              className="grid min-w-[360px] grid-flow-col grid-rows-7 gap-1"
              style={{
                gridTemplateColumns: `repeat(${Math.max(1, Math.ceil((days.length + (days.length ? (new Date(days[0].startMs).getDay() + 6) % 7 : 0)) / 7))}, minmax(5px,1fr))`,
              }}
            >
              {Array.from(
                {
                  length: days.length
                    ? (new Date(days[0].startMs).getDay() + 6) % 7
                    : 0,
                },
                (_, index) => (
                  <span key={`empty-${index}`} aria-hidden />
                ),
              )}
              {calendar.map(renderCell)}
            </div>
          </div>
        </>
      )}
      <div className="mt-3 flex flex-wrap items-center justify-between gap-2 text-note text-ink-muted">
        <p aria-live="polite">
          {description
            ? (shown[Math.min(focused, shown.length - 1)]?.label ?? "—")
            : busiest
              ? t("usageExperience.busiest", { value: busiest.label })
              : "—"}
        </p>
        <span className="inline-flex items-center gap-1" aria-hidden>
          {t("usageExperience.less")}
          {heatColors.map((color) => (
            <i key={color} className={`h-3 w-3 rounded-sm ${color}`} />
          ))}
          {t("usageExperience.more")}
        </span>
      </div>
      <p className="mt-2 text-note text-ink-faint">
        {t("usageExperience.activeDefinition")}
      </p>
    </section>
  );
}
