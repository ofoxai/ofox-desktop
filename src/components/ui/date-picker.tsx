import { useId, useLayoutEffect, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { CalendarDays, ChevronLeft, ChevronRight } from "lucide-react";
import { Button } from "./button";
import { Input } from "./input";
import {
  Popover,
  PopoverAnchor,
  PopoverContent,
  PopoverTrigger,
} from "./popover";
import { cn } from "@/lib/utils";
import { formatCalendarDate, parseCalendarDate } from "@/lib/calendarDate";

interface DatePickerProps {
  value: string;
  onChange: (value: string) => void;
  label: string;
  rangeStart?: string;
  rangeEnd?: string;
  invalid?: boolean;
  describedBy?: string;
}

function shiftMonth(date: Date, offset: number) {
  const next = new Date(date.getFullYear(), date.getMonth() + offset, 1);
  const last = new Date(next.getFullYear(), next.getMonth() + 1, 0).getDate();
  next.setDate(Math.min(date.getDate(), last));
  return next;
}

/** A local calendar day, with editable ISO text and no platform date popup. */
export function DatePicker({
  value,
  onChange,
  label,
  rangeStart,
  rangeEnd,
  invalid,
  describedBy,
}: DatePickerProps) {
  const { t, i18n } = useTranslation();
  const id = useId();
  const [open, setOpen] = useState(false);
  const [cursor, setCursor] = useState(() => formatCalendarDate(new Date()));
  const selected = parseCalendarDate(value);
  const focused = parseCalendarDate(cursor)!;
  const [month, setMonth] = useState(() => new Date());
  const buttons = useRef(new Map<string, HTMLButtonElement>());
  const focusPending = useRef(false);
  const locale = i18n.resolvedLanguage || i18n.language || "en";
  const monthLabel = new Intl.DateTimeFormat(locale, {
    year: "numeric",
    month: "long",
  }).format(month);
  const today = formatCalendarDate(new Date());
  const first = new Date(month.getFullYear(), month.getMonth(), 1);
  first.setDate(1 - first.getDay());
  const days = Array.from({ length: 42 }, (_, index) => {
    const day = new Date(first);
    day.setDate(first.getDate() + index);
    return day;
  });
  const start = parseCalendarDate(rangeStart ?? "");
  const end = parseCalendarDate(rangeEnd ?? "");

  useLayoutEffect(() => {
    if (open && focusPending.current) {
      buttons.current.get(cursor)?.focus();
      focusPending.current = false;
    }
  }, [cursor, month, open]);

  const move = (date: Date, focus = false) => {
    if (date.getFullYear() < 100 || date.getFullYear() > 9999) return;
    focusPending.current = focus;
    setCursor(formatCalendarDate(date));
    setMonth(new Date(date.getFullYear(), date.getMonth(), 1));
  };
  const pick = (date: Date) => {
    onChange(formatCalendarDate(date));
    setOpen(false);
  };

  return (
    <div className="grid min-w-0 gap-1.5">
      <label htmlFor={id} className="text-note font-medium text-ink-muted">
        {label}
      </label>
      <Popover
        open={open}
        onOpenChange={(next) => {
          if (next) move(selected ?? new Date());
          setOpen(next);
        }}
      >
        <PopoverAnchor asChild>
          <div className="relative">
            <Input
              id={id}
              type="text"
              inputMode="numeric"
              autoComplete="off"
              placeholder="YYYY-MM-DD"
              value={value}
              aria-invalid={invalid || undefined}
              aria-describedby={describedBy}
              onChange={(event) => onChange(event.target.value)}
              className="h-8 min-w-0 pr-9 text-sub tabular-nums aria-[invalid=true]:border-warn"
            />
            <PopoverTrigger asChild>
              <Button
                type="button"
                variant="icon"
                aria-label={t("datePicker.open", { label })}
                className="absolute right-1 top-1/2 h-6 w-6 -translate-y-1/2 data-[state=open]:bg-brand-soft data-[state=open]:text-brand-ink"
              >
                <CalendarDays aria-hidden className="h-3.5 w-3.5" />
              </Button>
            </PopoverTrigger>
          </div>
        </PopoverAnchor>
        <PopoverContent
          align="start"
          sideOffset={8}
          aria-label={t("datePicker.open", { label })}
          className="w-[286px] max-w-[calc(100vw-1rem)] overflow-hidden border border-line p-3"
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            buttons.current.get(cursor)?.focus();
          }}
        >
          <div className="mb-3 flex items-center justify-between">
            <span
              className="px-1 text-ui font-semibold text-ink"
              aria-live="polite"
            >
              {monthLabel}
            </span>
            <div className="flex gap-1">
              {([-1, 1] as const).map((offset) => (
                <Button
                  key={offset}
                  type="button"
                  variant="icon"
                  aria-label={t(
                    offset < 0
                      ? "datePicker.previousMonth"
                      : "datePicker.nextMonth",
                  )}
                  onClick={() => move(shiftMonth(focused, offset))}
                >
                  {offset < 0 ? (
                    <ChevronLeft aria-hidden className="h-3.5 w-3.5" />
                  ) : (
                    <ChevronRight aria-hidden className="h-3.5 w-3.5" />
                  )}
                </Button>
              ))}
            </div>
          </div>
          <div role="grid" aria-label={monthLabel}>
            <div
              role="row"
              className="mb-1 grid grid-cols-7 text-center text-note text-ink-muted"
            >
              {days.slice(0, 7).map((date) => (
                <span role="columnheader" key={date.getDay()} className="py-1">
                  {new Intl.DateTimeFormat(locale, {
                    weekday: "narrow",
                  }).format(date)}
                </span>
              ))}
            </div>
            {Array.from({ length: 6 }, (_, week) => (
              <div role="row" key={week} className="grid grid-cols-7">
                {days.slice(week * 7, week * 7 + 7).map((date) => {
                  const key = formatCalendarDate(date);
                  const endpoint = key === rangeStart || key === rangeEnd;
                  const inRange = !!(
                    start &&
                    end &&
                    start <= end &&
                    date >= start &&
                    date <= end
                  );
                  const isSelected = key === value;
                  return (
                    <div
                      role="gridcell"
                      aria-selected={isSelected || inRange}
                      key={key}
                      className={cn(
                        "my-0.5 flex justify-center",
                        inRange && "bg-brand-soft",
                        key === rangeStart && "rounded-l-ctl",
                        key === rangeEnd && "rounded-r-ctl",
                      )}
                    >
                      <button
                        type="button"
                        ref={(node) => {
                          if (node) buttons.current.set(key, node);
                          else buttons.current.delete(key);
                        }}
                        tabIndex={cursor === key ? 0 : -1}
                        aria-label={new Intl.DateTimeFormat(locale, {
                          year: "numeric",
                          month: "long",
                          day: "numeric",
                          weekday: "long",
                        }).format(date)}
                        aria-current={key === today ? "date" : undefined}
                        aria-pressed={isSelected}
                        onFocus={() => setCursor(key)}
                        onClick={() => pick(date)}
                        onKeyDown={(event) => {
                          const next = new Date(date);
                          const steps: Record<string, number> = {
                            ArrowLeft: -1,
                            ArrowRight: 1,
                            ArrowUp: -7,
                            ArrowDown: 7,
                          };
                          const step = steps[event.key];
                          if (step != null) next.setDate(next.getDate() + step);
                          else if (event.key === "Home")
                            next.setDate(next.getDate() - next.getDay());
                          else if (event.key === "End")
                            next.setDate(next.getDate() + 6 - next.getDay());
                          else if (
                            event.key === "PageUp" ||
                            event.key === "PageDown"
                          ) {
                            event.preventDefault();
                            move(
                              shiftMonth(
                                date,
                                (event.key === "PageUp" ? -1 : 1) *
                                  (event.shiftKey ? 12 : 1),
                              ),
                              true,
                            );
                            return;
                          } else return;
                          event.preventDefault();
                          move(next, true);
                        }}
                        className={cn(
                          "relative flex h-8 w-8 items-center justify-center rounded-ctl text-ctl tabular-nums transition-colors hover:bg-pill-hover focus-visible:z-10",
                          date.getMonth() !== month.getMonth()
                            ? "text-ink-muted"
                            : "text-ink-2",
                          inRange && "text-brand-ink hover:bg-brand/15",
                          key === today &&
                            "font-semibold after:absolute after:bottom-0.5 after:h-0.5 after:w-0.5 after:rounded-full after:bg-current",
                          endpoint && "font-medium text-brand-ink",
                          isSelected &&
                            "bg-brand font-semibold text-brand-on shadow-sm hover:bg-brand",
                        )}
                      >
                        {date.getDate()}
                      </button>
                    </div>
                  );
                })}
              </div>
            ))}
          </div>
          <div className="mt-2 flex items-center justify-between border-t border-line pt-2">
            <span className="px-1 text-note tabular-nums text-ink-muted">
              {value || "YYYY-MM-DD"}
            </span>
            <Button
              type="button"
              variant="ghost"
              size="sm"
              className="text-brand-ink"
              onClick={() => pick(new Date())}
            >
              {t("datePicker.today")}
            </Button>
          </div>
        </PopoverContent>
      </Popover>
    </div>
  );
}
