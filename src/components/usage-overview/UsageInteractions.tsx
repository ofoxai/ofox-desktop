import { useState } from "react";
import { useTranslation } from "react-i18next";
import { SegmentedControl } from "@/components/ui/segmented";
import { Button } from "@/components/ui/button";
import { fmtTokens } from "@/lib/format";
import { TOOL_META } from "@/config/toolMeta";
import type {
  AnalyticsDistribution,
  AnalyticsInteractions,
} from "@/types/sessionAnalytics";

// Separate categorical hues; the orange brand stays reserved for primary data.
const colors: Record<string, string> = {
  Bash: "#df8948",
  Read: "#32a985",
  Edit: "#b17bd7",
  Write: "#8f69bf",
  Grep: "#d56f93",
  Glob: "#c6a339",
  Task: "#58a7b2",
  Tool: "#a286bc",
  Other: "#999da5",
};
const color = (category: string) => colors[category] ?? colors.Other;
const card = "min-w-0 rounded-list border border-line bg-card p-4";
const heading = "text-sub font-medium text-ink-muted";

function Coverage({ data }: { data?: AnalyticsInteractions }) {
  const { t } = useTranslation();
  if (!data || data.coverage.state === "unavailable") return null;
  if (data.coverage.state === "complete") return null;
  return (
    <p className="mt-3 text-micro leading-relaxed text-ink-muted">
      {t("usageInteractions.coverage", {
        count: data.coverage.eligibleSessions,
        excluded: data.coverage.excludedSessions,
      })}
      {data.coverage.unquantifiedSourceGaps
        ? ` · ${t("usageInteractions.sourceGaps")}`
        : ""}
    </p>
  );
}
function Empty({
  unavailable,
  shape = false,
}: {
  unavailable: boolean;
  shape?: boolean;
}) {
  const { t } = useTranslation();
  return (
    <p className="flex min-h-[160px] items-center justify-center text-note text-ink-muted">
      {t(
        shape
          ? "usageInteractions.noShape"
          : unavailable
            ? "usageInteractions.unavailable"
            : "usageInteractions.noCalls",
      )}
    </p>
  );
}
export function UsageSessionShape({
  distributions,
  interactions,
}: {
  distributions?: AnalyticsDistribution[];
  interactions?: AnalyticsInteractions;
}) {
  const { t, i18n } = useTranslation();
  const [metric, setMetric] = useState<
    "messages" | "activeMs" | "toolCallsPerPrompt"
  >("messages");
  const distribution =
    metric === "toolCallsPerPrompt"
      ? interactions?.autonomy
      : distributions?.find((d) => d.metric === metric);
  const max = Math.max(1, ...(distribution?.counts ?? []));
  const range = (start: number, end?: number) => {
    if (metric === "activeMs") {
      if (start === 0 && end === 1) return "0";
      if (start === 1 && end === 60000) return "<1";
      start = Math.ceil(start / 60000);
      end = end == null ? undefined : Math.ceil(end / 60000);
    }
    if (start === 0 && end === 1)
      return metric === "toolCallsPerPrompt" ? "<1" : "0";
    return end == null
      ? `${start}+`
      : end - 1 === start
        ? String(start)
        : `${start}–${end - 1}`;
  };
  return (
    <section className={card} aria-label={t("usageInteractions.shape")}>
      <header className="flex flex-wrap items-center justify-between gap-2">
        <h2 className={heading}>{t("usageInteractions.shape")}</h2>
        <SegmentedControl
          aria-label={t("usageInteractions.shapeMetric")}
          value={metric}
          onChange={setMetric}
          options={[
            { value: "messages", label: t("usageInteractions.messages") },
            { value: "activeMs", label: t("usageInteractions.length") },
            {
              value: "toolCallsPerPrompt",
              label: t("usageInteractions.toolCalls"),
            },
          ]}
        />
      </header>
      {distribution && distribution.samples > 0 ? (
        <>
          <div
            className="mb-2 mt-5 flex h-[184px] items-end gap-2"
            role="list"
            aria-label={t(`usageInteractions.axis.${metric}`)}
          >
            {distribution.counts.map((count, index) => {
              const label = range(
                distribution.edges[index],
                distribution.edges[index + 1],
              );
              return (
                <div
                  key={index}
                  role="listitem"
                  aria-label={t("usageExperience.distributionBucket", {
                    range: label,
                    count,
                  })}
                  className="flex h-full min-w-0 flex-1 flex-col justify-end text-center"
                  title={t("usageExperience.distributionBucket", {
                    range: label,
                    count,
                  })}
                >
                  <div className="relative flex min-h-0 flex-1 items-end justify-center border-b border-line-2 px-[12%]">
                    <span
                      aria-hidden
                      className={`relative w-full rounded-t-lg ${count === max ? "bg-brand" : "bg-brand/45"}`}
                      style={{
                        height: `${(count / max) * 82}%`,
                        minHeight: count > 0 ? 3 : 0,
                      }}
                    >
                      <span className="absolute -top-6 left-1/2 -translate-x-1/2 text-note font-medium tabular-nums text-ink-2">
                        {count.toLocaleString(i18n.language)}
                      </span>
                    </span>
                  </div>
                  <span className="pt-2 text-micro tabular-nums text-ink-muted">
                    {label}
                  </span>
                </div>
              );
            })}
          </div>
          <p className="mt-4 text-micro leading-relaxed text-ink-muted">
            {t(`usageInteractions.axis.${metric}`)}
          </p>
          <p className="mt-2 text-micro text-ink-muted">
            {t("usageInteractions.samples", {
              count: distribution.samples,
              excluded: distribution.excludedSessions,
            })}
            {distribution.unquantifiedSourceGaps
              ? ` · ${t("usageInteractions.sourceGaps")}`
              : ""}
          </p>
        </>
      ) : (
        <Empty
          shape
          unavailable={
            !distribution ||
            distribution.excludedSessions > 0 ||
            distribution.unquantifiedSourceGaps
          }
        />
      )}
    </section>
  );
}
export function UsageSkillInsights({ data }: { data?: AnalyticsInteractions }) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const unavailable = !data || data.coverage.state === "unavailable";
  const skills = unavailable ? undefined : data.skills;
  const max = Math.max(1, ...(skills?.top.map((row) => row.calls) ?? []));
  const title = t("usageInteractions.skills");
  return (
    <section className={card} aria-label={title}>
      <header className="mb-4 flex flex-wrap items-center justify-between gap-2">
        <h2 className={heading}>{title}</h2>
        {skills &&
          (skills.calls > 0 || data?.coverage.state === "complete") && (
            <span className="text-micro tabular-nums text-ink-muted">
              {t("usageInteractions.skillTotal", {
                calls: `${fmtTokens(skills.calls)}${data?.coverage.state === "partial" ? "+" : ""}`,
                count: skills.count,
              })}
            </span>
          )}
      </header>
      {skills?.top.length ? (
        <div className="space-y-4">
          {skills.top.slice(0, expanded ? undefined : 6).map((skill) => {
            const agents = Object.entries(skill.agents)
              .sort((a, b) => b[1] - a[1])
              .map(
                ([id, calls]) =>
                  `${TOOL_META[id]?.label ?? id} ${Math.round((calls / Math.max(1, skill.calls)) * 100)}%`,
              )
              .join(" · ");
            const date = new Date(`${skill.last}T12:00:00`);
            const last = Number.isNaN(date.getTime())
              ? skill.last
              : date.toLocaleDateString(i18n.language, {
                  month: "short",
                  day: "numeric",
                });
            const detail = [
              t("usageInteractions.skillSessions", {
                count: skill.sessions,
                date: last,
              }),
              agents,
              ...skill.projects.map((project) => project.name),
            ].join(" · ");
            return (
              <div key={skill.name}>
                <div className="grid grid-cols-[minmax(0,1fr)_minmax(55px,28%)_auto] items-center gap-3">
                  <span
                    className="truncate text-sub font-medium"
                    title={skill.name}
                  >
                    {skill.name}
                  </span>
                  <span
                    className="h-1.5 overflow-hidden rounded-pill bg-pill"
                    aria-hidden
                  >
                    <span
                      className="block h-full rounded-pill bg-brand/65"
                      style={{ width: `${(skill.calls / max) * 100}%` }}
                    />
                  </span>
                  <b className="min-w-5 text-right text-note font-medium tabular-nums text-ink-muted">
                    {fmtTokens(skill.calls)}
                  </b>
                </div>
                <p
                  className="mt-1 truncate text-micro text-ink-muted"
                  title={detail}
                >
                  {detail}
                </p>
              </div>
            );
          })}
          {skills.top.length > 6 && (
            <Button
              variant="ghost"
              className="h-6 px-0 text-micro"
              onClick={() => setExpanded(!expanded)}
            >
              {t(
                expanded ? "usageInteractions.less" : "usageInteractions.more",
              )}
            </Button>
          )}
        </div>
      ) : (
        <Empty
          unavailable={unavailable || data?.coverage.state === "partial"}
        />
      )}
      <Coverage data={data} />
    </section>
  );
}
export function UsageToolInsights({ data }: { data?: AnalyticsInteractions }) {
  const { t, i18n } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const unavailable = !data || data.coverage.state === "unavailable";
  const tools = unavailable ? undefined : data.tools;
  const topMax = Math.max(1, ...(tools?.top.map((row) => row.calls) ?? []));
  const weekMax = Math.max(
    1,
    ...(tools?.weeks.map((week) =>
      Object.values(week.calls).reduce((sum, value) => sum + value, 0),
    ) ?? []),
  );
  const number = (value: number) => value.toLocaleString(i18n.language);
  const categoryLabel = (name: string) =>
    name === "Other" ? t("usageInteractions.other") : name;
  return (
    <section className={card} aria-label={t("usageInteractions.tools")}>
      <header className="mb-5 flex flex-wrap items-center justify-between gap-2">
        <h2 className={heading}>{t("usageInteractions.tools")}</h2>
        {tools && (tools.calls > 0 || data?.coverage.state === "complete") && (
          <span className="text-micro tabular-nums text-ink-muted">
            {t("usageInteractions.toolTotal", {
              calls: `${fmtTokens(tools.calls)}${data?.coverage.state === "partial" ? "+" : ""}`,
              count: tools.sessions,
            })}
          </span>
        )}
      </header>
      {tools?.top.length ? (
        <div className="grid min-w-0 gap-6 min-[900px]:grid-cols-2">
          <div className="space-y-3">
            {tools.top.slice(0, expanded ? undefined : 8).map((tool) => (
              <div
                key={tool.name}
                className="grid grid-cols-[minmax(70px,1fr)_minmax(45px,1fr)_auto] items-center gap-3"
              >
                <span className="flex min-w-0 items-center gap-2 text-note">
                  <span
                    className="h-2 w-2 shrink-0 rounded-full"
                    aria-hidden
                    style={{ background: color(tool.category) }}
                  />
                  <span className="truncate" title={tool.name}>
                    {tool.name}
                  </span>
                </span>
                <span
                  className="h-1.5 overflow-hidden rounded-pill bg-pill"
                  aria-hidden
                >
                  <span
                    className="block h-full rounded-pill"
                    style={{
                      width: `${(tool.calls / topMax) * 100}%`,
                      background: color(tool.category),
                    }}
                  />
                </span>
                <span
                  className="flex items-baseline justify-end gap-2 text-micro tabular-nums text-ink-muted"
                  title={t("usageInteractions.toolDetail", {
                    calls: number(tool.calls),
                    count: tool.sessions,
                  })}
                >
                  <b className="text-note font-medium text-ink-2">
                    {fmtTokens(tool.calls)}
                  </b>
                  <span className="hidden min-[1100px]:inline">
                    {t("usageInteractions.sessions", { count: tool.sessions })}
                  </span>
                  <span className="w-8 text-right text-ink-muted">
                    {Math.round((tool.calls / Math.max(1, tools.calls)) * 100)}%
                  </span>
                </span>
              </div>
            ))}
            {tools.top.length > 8 && (
              <Button
                variant="ghost"
                className="h-6 px-0 text-micro"
                onClick={() => setExpanded(!expanded)}
              >
                {t(
                  expanded
                    ? "usageInteractions.less"
                    : "usageInteractions.more",
                )}
              </Button>
            )}
          </div>
          <div className="min-w-0">
            <div
              className="flex h-3 overflow-hidden rounded-pill"
              aria-label={t("usageInteractions.composition")}
            >
              {tools.categories.map((cat) => (
                <span
                  key={cat.name}
                  title={`${categoryLabel(cat.name)} · ${number(cat.calls)}`}
                  style={{
                    width: `${(cat.calls / tools.calls) * 100}%`,
                    background: color(cat.name),
                  }}
                  className="border-r border-card last:border-0"
                />
              ))}
            </div>
            <div className="mt-3 flex flex-wrap gap-x-4 gap-y-2 text-micro">
              {tools.categories.map((cat) => (
                <span
                  key={cat.name}
                  className="inline-flex items-center gap-1.5 text-ink-muted"
                >
                  <span
                    className="h-2 w-2 rounded-full"
                    aria-hidden
                    style={{ background: color(cat.name) }}
                  />
                  {categoryLabel(cat.name)}
                  <b className="font-medium tabular-nums text-ink-2">
                    {fmtTokens(cat.calls)}
                  </b>
                </span>
              ))}
            </div>
            <h3 className="mb-2 mt-5 text-micro font-medium text-ink-muted">
              {t("usageInteractions.weekly")}
            </h3>
            <div
              className="flex h-[100px] items-end gap-1"
              aria-label={t("usageInteractions.weekly")}
            >
              {tools.weeks.map((week) => (
                <div
                  key={week.start}
                  className="flex h-full min-w-0 flex-1 items-end"
                  title={`${week.start} · ${number(Object.values(week.calls).reduce((s, v) => s + v, 0))}`}
                >
                  <div
                    className="flex w-full flex-col-reverse overflow-hidden rounded-t"
                    style={{
                      height: `${(Object.values(week.calls).reduce((s, v) => s + v, 0) / weekMax) * 100}%`,
                    }}
                  >
                    {tools.categories.map((cat) => (
                      <span
                        key={cat.name}
                        style={{
                          background: color(cat.name),
                          flex: week.calls[cat.name] ?? 0,
                          minHeight: week.calls[cat.name] ? 1 : 0,
                        }}
                      />
                    ))}
                  </div>
                </div>
              ))}
            </div>
            <div className="mt-1 flex justify-between text-micro tabular-nums text-ink-muted">
              <span>{tools.weeks[0]?.start.slice(5)}</span>
              <span>
                {tools.weeks.length > 1
                  ? tools.weeks.at(-1)?.start.slice(5)
                  : ""}
              </span>
            </div>
          </div>
        </div>
      ) : (
        <Empty
          unavailable={unavailable || data?.coverage.state === "partial"}
        />
      )}
      <Coverage data={data} />
      {!!tools?.top.length && (
        <details className="mt-3 text-micro text-ink-muted">
          <summary className="cursor-pointer">
            {t("usageInteractions.dataTable")}
          </summary>
          <div className="mt-3 max-h-[220px] overflow-auto">
            <table
              className="w-full text-left"
              aria-label={t("usageInteractions.weekly")}
            >
              <thead>
                <tr>
                  <th>{t("usageWorkbench.date")}</th>
                  {tools.categories.map((cat) => (
                    <th key={cat.name} className="px-2 text-right">
                      {categoryLabel(cat.name)}
                    </th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {tools.weeks.map((week) => (
                  <tr key={week.start}>
                    <td className="py-1">{week.start}</td>
                    {tools.categories.map((cat) => (
                      <td
                        key={cat.name}
                        className="px-2 text-right tabular-nums"
                      >
                        {number(week.calls[cat.name] ?? 0)}
                      </td>
                    ))}
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </details>
      )}
    </section>
  );
}
