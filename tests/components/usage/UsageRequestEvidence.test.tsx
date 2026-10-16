import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, afterEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { UsageRequests } from "@/components/usage-overview/UsageRequests";
import { queryClient } from "@/lib/query/queryClient";
import { createUsageQuery } from "@/lib/usageWorkbench";
import { requestMeasurements } from "@/lib/usageRequestEvidence";
import type { UsageEstimate } from "@/types/usageQuery";
import type { RequestLog, UsageRequestMeasurements } from "@/types/usage";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: vi.fn() }));
const query = createUsageQuery({ period: "all" }, {}, 1000);
const free: UsageEstimate = {
  knownCostUsd: "0",
  availability: "free",
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
};
const row: RequestLog = {
  requestId: "native",
  providerId: "_workbuddy_session",
  appType: "workbuddy",
  model: "test-model",
  costMultiplier: "1",
  inputTokens: 10,
  outputTokens: 20,
  cacheReadTokens: 30,
  cacheCreationTokens: 0,
  inputCostUsd: "0",
  outputCostUsd: "0",
  cacheReadCostUsd: "0",
  cacheCreationCostUsd: "0",
  totalCostUsd: "0",
  latencyMs: 0,
  statusCode: 200,
  createdAt: 1,
  isStreaming: false,
  dataSource: "workbuddy_session",
  observedStatusCode: null,
  observedLatencyMs: null,
  observedFirstTokenMs: null,
  currentEstimate: free,
};
const receipt = {
  appliedQuery: query,
  queryFingerprint: "q",
  snapshotVersion: "v",
  coverage: {
    state: "complete",
    reasons: [],
    timeBasis: "eventTime",
    detailStartMs: 0,
    detailEndMs: 1000,
    aggregateOnly: false,
    excludedRollupDays: 0,
    sourceIds: ["workbuddy_session"],
  },
};
const props = {
  query,
  apex: "ofox.ai",
  active: true,
  sourceIds: ["workbuddy_session", "codex_session", "proxy", "future_source"],
  updateVersion: 0,
  canExport: false,
  onLatest: vi.fn(),
};
let records: RequestLog[];
let measurements: UsageRequestMeasurements | undefined;
beforeEach(async () => {
  queryClient.clear();
  mocks.invoke.mockReset();
  for (const [lang, resource] of Object.entries({ zh, en, ja }))
    i18n.addResourceBundle(lang, "translation", resource, true, true);
  await i18n.changeLanguage("zh");
  Element.prototype.scrollIntoView = vi.fn();
  records = [{ ...row }];
  measurements = undefined;
  mocks.invoke.mockImplementation(async (command, args) => {
    if (command === "get_request_logs")
      return {
        ...receipt,
        appliedQuery: args.query,
        requestFilters: args.requestFilters ?? {},
        data: records,
        total: records.length,
        page: args.page,
        pageSize: 20,
        snapshotId: "s",
        measurements,
        summary: { requests: records.length, totalTokens: 30, estimate: free },
      };
    if (command === "get_request_detail")
      return {
        ...records.find((r) => r.requestId === args.requestId),
        receipt,
      };
    throw new Error(command);
  });
});
afterEach(async () => {
  await i18n.changeLanguage("zh");
});

it.each([
  "session_log",
  "codex_session",
  "gemini_session",
  "opencode_session",
  "workbuddy_session",
  "pi_session",
  "dsh_session",
  "openclaw_session",
])(
  "labels %s as imported usage without implying missing usage or success",
  async (source) => {
    records = [{ ...row, dataSource: source }];
    render(<UsageRequests {...props} />);
    const item = await screen.findByRole("button", { name: "查看请求 native" });
    expect(item).toHaveTextContent("本机会话导入");
    expect(item).not.toHaveTextContent(source);
    expect(item).not.toHaveTextContent("未记录");
    expect(item).not.toHaveTextContent("HTTP 200");
    expect(item).toHaveTextContent("30");
    expect(item).toHaveTextContent("≈$0");
    expect(within(item).getByText("本机会话导入")).toHaveAttribute(
      "title",
      expect.stringMatching(/本地历史记录读取/),
    );
  },
);

it.each([
  ["zh", "本机会话导入"],
  ["en", "Local session import"],
  ["ja", "ローカル会話の取り込み"],
])("localizes source names in %s", async (lang, label) => {
  await i18n.changeLanguage(lang);
  render(<UsageRequests {...props} />);
  expect(await screen.findByText(label)).toBeVisible();
  expect(screen.queryByText(/workbuddy_session/)).toBeNull();
});

it("groups missing timing, explains absent status, and hides raw IDs in technical details", async () => {
  render(<UsageRequests {...props} />);
  const trigger = await screen.findByRole("button", {
    name: "查看请求 native",
  });
  trigger.focus();
  await userEvent.keyboard("{Enter}");
  const dialog = await screen.findByRole("dialog");
  expect(
    await within(dialog).findByText("原始记录未提供请求状态"),
  ).toBeVisible();
  expect(
    within(dialog).getByText("原始记录未提供：耗时、首 Token 用时。"),
  ).toBeVisible();
  expect(within(dialog).queryByText("未记录")).toBeNull();
  expect(within(dialog).getByText("workbuddy_session")).not.toBeVisible();
  expect(within(dialog).getByText("≈$0")).toBeVisible();
  expect(within(dialog).getByText("现价估算，以账户账单为准。")).toBeVisible();
  const tech = within(dialog).getByText("技术详情");
  tech.focus();
  await userEvent.click(tech);
  expect(within(dialog).getByText("workbuddy_session")).toBeVisible();
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(trigger).toHaveFocus());
});

it.each([200, 429])(
  "shows only measured HTTP %i and genuine zero timing",
  async (status) => {
    records = [
      {
        ...row,
        dataSource: "proxy",
        observedStatusCode: status,
        observedLatencyMs: 0,
        observedFirstTokenMs: 0,
      },
    ];
    render(<UsageRequests {...props} />);
    const item = await screen.findByRole("button", { name: "查看请求 native" });
    expect(item).toHaveTextContent(`HTTP ${status}`);
    await userEvent.click(item);
    const d = await screen.findByRole("dialog");
    expect(await within(d).findByText(`HTTP ${status}`)).toBeVisible();
    expect(within(d).getAllByText("0 ms")).toHaveLength(2);
    expect(within(d).queryByText("原始记录未提供请求状态")).toBeNull();
  },
);

it("keeps free, unknown and partial current estimates distinct from saved zero placeholders", async () => {
  records = [
    { ...row, requestId: "free" },
    {
      ...row,
      requestId: "unknown",
      currentEstimate: {
        ...free,
        availability: "unknown",
        knownCostUsd: null,
        unpricedModelIds: ["m"],
        unpricedRecordCount: 1,
      },
    },
    {
      ...row,
      requestId: "partial",
      currentEstimate: {
        ...free,
        availability: "partial",
        knownCostUsd: "0.01",
        unpricedRecordCount: 1,
      },
    },
  ];
  render(<UsageRequests {...props} />);
  expect(
    await screen.findByRole("button", { name: "查看请求 free" }),
  ).toHaveTextContent("≈$0");
  const unknown = screen.getByRole("button", { name: "查看请求 unknown" });
  expect(unknown).toHaveTextContent("—");
  expect(unknown).not.toHaveTextContent("≈$0");
  expect(
    screen.getByRole("button", { name: "查看请求 partial" }),
  ).toHaveTextContent("≈$0.010+");
  await userEvent.click(unknown);
  const d = await screen.findByRole("dialog");
  expect(await within(d).findByText("—")).toBeVisible();
});

it("uses no available samples instead of zero percent without measured status and retains tokens", async () => {
  measurements = {
    statusSamples: 0,
    failedSamples: 0,
    failureRate: 0,
    latencySamples: 0,
    avgLatencyMs: null,
    firstTokenSamples: 0,
    avgFirstTokenMs: null,
    generationSpeedSamples: 0,
    avgGenerationTokensPerSecond: null,
  };
  render(<UsageRequests {...props} />);
  const m = await screen.findByRole("region", { name: "请求测量" });
  expect(m).toHaveTextContent("暂无可用样本");
  expect(m).not.toHaveTextContent("0%");
  expect(
    screen.getByRole("button", { name: "查看请求 native" }),
  ).toHaveTextContent("30");
});

it("offers a friendly source filter and explains no measured status is not failure", async () => {
  render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: "查看请求 native" });
  await userEvent.click(screen.getByRole("combobox", { name: "记录来源" }));
  expect(
    screen.getByRole("option", { name: /WorkBuddy · 本机会话导入/ }),
  ).toBeVisible();
  expect(screen.queryByText("future_source")).toBeNull();
  await userEvent.keyboard("{Escape}");
  const status = screen.getByRole("combobox", { name: "状态" });
  status.focus();
  await userEvent.keyboard("{ArrowDown}");
  const missing = screen.getByRole("option", { name: /无实测状态/ });
  expect(missing).toHaveTextContent("不代表失败");
  await userEvent.click(missing);
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_request_logs",
      expect.objectContaining({ requestFilters: { statusKind: "unrecorded" } }),
    ),
  );
});

it("uses a neutral future-source name without treating a source suffix as proof", async () => {
  records = [
    { ...row, dataSource: "future_session", observedStatusCode: undefined },
  ];
  render(<UsageRequests {...props} />);
  const item = await screen.findByRole("button", { name: "查看请求 native" });
  expect(item).toHaveTextContent("其他来源记录");
  expect(item).not.toHaveTextContent("future_session");
  expect(item).not.toHaveTextContent("HTTP 200");
});

it.each([0, 99, 600, -1, 200.5, Number.NaN])(
  "rejects invalid measured HTTP status %s",
  (code) => {
    expect(
      requestMeasurements({ ...row, observedStatusCode: code }).status,
    ).toBeNull();
  },
);
it("uses legacy measurements only for retained request records and never revives explicit null", () => {
  const {
    observedStatusCode,
    observedLatencyMs,
    observedFirstTokenMs,
    ...legacy
  } = row;
  expect(
    requestMeasurements({ ...legacy, dataSource: "proxy", latencyMs: 0 }),
  ).toEqual({ status: 200, latency: 0, firstToken: null });
  expect(
    requestMeasurements({
      ...row,
      dataSource: "proxy",
      statusCode: 200,
      latencyMs: 999,
      firstTokenMs: 999,
    }),
  ).toEqual({ status: null, latency: null, firstToken: null });
  expect(
    requestMeasurements({
      ...legacy,
      dataSource: "workbuddy_session",
      latencyMs: 999,
    }),
  ).toEqual({ status: null, latency: null, firstToken: null });
});
it.each(["constructor", "__proto__", "toString"])(
  "does not interpret unknown source %s as a known adapter",
  async (source) => {
    records = [{ ...row, dataSource: source }];
    render(<UsageRequests {...props} />);
    expect(
      await screen.findByRole("button", { name: "查看请求 native" }),
    ).toHaveTextContent("其他来源记录");
  },
);
