import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageRequests } from "@/components/usage-overview/UsageRequests";
import { queryClient } from "@/lib/query/queryClient";
import { createUsageQuery } from "@/lib/usageWorkbench";
const mocks = vi.hoisted(() => ({ invoke: vi.fn(), save: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ save: mocks.save }));
const query = createUsageQuery({ period: "all" }, {}, 1000);
const estimate = {
  knownCostUsd: "0",
  availability: "free",
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
};
const row = {
  requestId: "native-1",
  providerId: "fixture",
  appType: "opencode",
  model: "model",
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
  dataSource: "opencode_session",
  observedStatusCode: null,
  observedLatencyMs: null,
  currentEstimate: estimate,
};
const receipt = (appliedQuery = query) => ({
  appliedQuery,
  queryFingerprint: "frozen-query",
  snapshotVersion: "version-1",
  coverage: {
    state: "complete",
    reasons: [],
    timeBasis: "eventTime",
    detailStartMs: 0,
    detailEndMs: 1000,
    aggregateOnly: false,
    excludedRollupDays: 0,
    sourceIds: ["opencode_session"],
  },
});
const props = {
  query,
  apex: "ofox.ai",
  active: true,
  sourceIds: ["proxy", "opencode_session"],
  updateVersion: 0,
  canExport: true,
  onLatest: vi.fn(),
};
beforeEach(() => {
  queryClient.clear();
  mocks.invoke.mockReset();
  mocks.save.mockReset().mockResolvedValue(null);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  Element.prototype.scrollIntoView = vi.fn();
  mocks.invoke.mockImplementation(async (command: string, args: any) => {
    if (command === "get_request_logs")
      return {
        ...receipt(args.query),
        requestFilters: args.requestFilters ?? {},
        data: [row],
        total: 45,
        page: args.page,
        pageSize: 20,
        snapshotId: args.snapshotId ?? "snapshot-1",
      };
    if (command === "get_request_detail") return { ...row, receipt: receipt() };
    if (command === "export_request_logs_csv")
      return {
        ...receipt(),
        csv: "id\r\n1\r\n",
        fileName: "usage.csv",
        recordCount: 45,
      };
    throw new Error(command);
  });
});
it("keeps a frozen snapshot across pages and resets the first request on filter changes", async () => {
  const view = render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  await userEvent.click(screen.getByRole("button", { name: "下一页" }));
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith(
      "get_request_logs",
      expect.objectContaining({ page: 1, snapshotId: "snapshot-1" }),
    ),
  );
  mocks.invoke.mockClear();
  const next = { ...query, toolIds: ["opencode"] };
  view.rerender(<UsageRequests {...props} query={next} />);
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalled());
  expect(
    mocks.invoke.mock.calls.find(([name]) => name === "get_request_logs")?.[1],
  ).toMatchObject({ query: next, page: 0 });
  expect(
    mocks.invoke.mock.calls.find(([name]) => name === "get_request_logs")?.[1],
  ).not.toHaveProperty("snapshotId");
});
it("opens detail by keyboard without presenting imported synthetic status as observed", async () => {
  render(<UsageRequests {...props} />);
  const trigger = await screen.findByRole("button", {
    name: /model/,
  });
  expect(trigger).toHaveTextContent("本机会话导入");
  expect(trigger).not.toHaveTextContent("未记录");
  trigger.focus();
  await userEvent.keyboard("{Enter}");
  const dialog = await screen.findByRole("dialog");
  expect(
    await within(dialog).findByText("原始记录未提供请求状态"),
  ).toBeVisible();
  expect(
    within(dialog).getByText(/原始记录未提供：.*首 Token 用时/),
  ).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("get_request_detail", {
    requestId: "native-1",
    snapshotId: "snapshot-1",
  });
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(trigger).toHaveFocus());
});

it("shows manufacturer API pricing as the estimate basis in frozen request details", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command: string, args: any) =>
    command === "get_request_detail"
      ? Promise.resolve({
          ...row,
          receipt: receipt(),
          currentEstimate: {
            ...estimate,
            knownCostUsd: "4",
            availability: "known",
            officialModelIds: ["claude-opus-5-5"],
          },
        })
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  await userEvent.click(await screen.findByRole("button", { name: /model/ }));
  const dialog = await screen.findByRole("dialog");
  expect(await within(dialog).findByText("厂商官方 API 标价")).toBeVisible();
  expect(within(dialog).getByText("估价依据")).toBeVisible();
});
it("distinguishes a detail fetch failure from a missing record", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "get_request_detail"
      ? Promise.reject(new Error("offline"))
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  await userEvent.click(await screen.findByRole("button", { name: /model/ }));
  expect(await screen.findByText("请求详情读取失败")).toBeVisible();
  expect(screen.queryByText("该快照中未找到请求")).toBeNull();
});
it("confirms the complete readable count and refuses a late export after its filters change", async () => {
  let release!: (value: unknown) => void;
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "export_request_logs_csv"
      ? new Promise((resolve) => {
          release = resolve;
        })
      : normal(command, args),
  );
  const view = render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  await userEvent.click(screen.getByRole("button", { name: "导出 CSV" }));
  expect(screen.getByRole("dialog")).toHaveTextContent("45 条可读明细");
  await userEvent.click(screen.getByRole("button", { name: "保存 CSV" }));
  view.rerender(
    <UsageRequests {...props} query={{ ...query, modelIds: ["other"] }} />,
  );
  release({ ...receipt(), fileName: "old.csv", csv: "old", recordCount: 45 });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(mocks.save).not.toHaveBeenCalled();
});
it("shows new-data notice without replacing an older page or detail", async () => {
  const view = render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  await userEvent.click(screen.getByRole("button", { name: "下一页" }));
  await screen.findByText("第 2 / 3 页");
  mocks.invoke.mockClear();
  view.rerender(<UsageRequests {...props} updateVersion={1} />);
  expect(screen.getByText(/当前分页保持原快照/)).toBeVisible();
  expect(screen.getByText("第 2 / 3 页")).toBeVisible();
  expect(mocks.invoke).not.toHaveBeenCalled();
});

it("drops the old page snapshot when the resolved region changes", async () => {
  const view = render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  await userEvent.click(screen.getByRole("button", { name: "下一页" }));
  await screen.findByText("第 2 / 3 页");
  mocks.invoke.mockClear();
  view.rerender(<UsageRequests {...props} apex="ofox.io" />);
  await waitFor(() => expect(mocks.invoke).toHaveBeenCalled());
  const next = mocks.invoke.mock.calls.find(
    ([name]) => name === "get_request_logs",
  )?.[1];
  expect(next).toMatchObject({ page: 0 });
  expect(next).not.toHaveProperty("snapshotId");
});

it("honors explicit unknown measurements on proxy details as well", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "get_request_detail"
      ? Promise.resolve({ ...row, dataSource: "proxy", receipt: receipt() })
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  await userEvent.click(await screen.findByRole("button", { name: /model/ }));
  const dialog = await screen.findByRole("dialog");
  expect(
    await within(dialog).findByText("原始记录未提供请求状态"),
  ).toBeVisible();
  expect(
    within(dialog).getByText(/原始记录未提供：.*首 Token 用时/),
  ).toBeVisible();
  expect(within(dialog).queryByText("200")).toBeNull();
});

it("never revives an abandoned export after the filters return to the old values", async () => {
  let release!: (value: unknown) => void;
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "export_request_logs_csv"
      ? new Promise((resolve) => {
          release = resolve;
        })
      : normal(command, args),
  );
  const view = render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  await userEvent.click(screen.getByRole("button", { name: "导出 CSV" }));
  await userEvent.click(screen.getByRole("button", { name: "保存 CSV" }));
  view.rerender(
    <UsageRequests {...props} query={{ ...query, modelIds: ["other"] }} />,
  );
  view.rerender(<UsageRequests {...props} />);
  release({ ...receipt(), fileName: "old.csv", csv: "old", recordCount: 45 });
  await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  expect(mocks.save).not.toHaveBeenCalled();
});

it("publishes new snapshot options after explicitly loading latest records", async () => {
  const onOptions = vi.fn();
  const normal = mocks.invoke.getMockImplementation()!;
  let modelIds = ["initial-model"];
  mocks.invoke.mockImplementation(async (command, args) =>
    command === "get_request_logs"
      ? {
          ...(await normal(command, args)),
          options: {
            toolIds: ["opencode"],
            modelIds,
            sourceIds: ["opencode_session"],
          },
        }
      : normal(command, args),
  );
  const view = render(<UsageRequests {...props} onOptions={onOptions} />);
  await waitFor(() =>
    expect(onOptions).toHaveBeenCalledWith(
      expect.objectContaining({ modelIds: ["initial-model"] }),
    ),
  );
  modelIds = ["initial-model", "new-model"];
  view.rerender(
    <UsageRequests {...props} onOptions={onOptions} updateVersion={1} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "查看最新记录" }));
  await waitFor(() =>
    expect(onOptions).toHaveBeenCalledWith(
      expect.objectContaining({ modelIds: ["initial-model", "new-model"] }),
    ),
  );
});

it("keeps unavailable price and unsupported coverage distinct from empty request data", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command, args) =>
    command === "get_request_logs"
      ? {
          ...(await normal(command, args)),
          data: [],
          total: 0,
          coverage: { ...receipt().coverage, state: "unsupported" },
          pricingCoverage: {
            policy: "current-ofox-base-v1",
            apex: "ofox.ai",
            catalogVersion: null,
            catalogUpdatedAtMs: null,
            stale: false,
            catalogState: "invalid",
            errorCode: "internal should not render",
            unpricedModelIds: [],
            unpricedRecordCount: null,
            additionalUnitsUnsupported: false,
          },
        }
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  expect(await screen.findByText(/价格目录不可用/)).toBeVisible();
  expect(
    screen.getByText("当前条件的统计暂不支持，无法据此判断没有用量。"),
  ).toBeVisible();
  expect(screen.queryByText("当前条件下没有可读的请求明细")).toBeNull();
  expect(screen.queryByText(/internal should not render/)).toBeNull();
});

it("shows the frozen complete-condition trend and measured sample denominators across pages", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation(async (command, args) =>
    command === "get_request_logs"
      ? {
          ...(await normal(command, args)),
          trend: {
            granularity: "day",
            series: [
              {
                start: 0,
                requests: 45,
                inputTokens: 450,
                outputTokens: 90,
                cacheReadTokens: 3000,
                cacheWriteTokens: 0,
                costUsd: 0,
                estimate,
              },
            ],
          },
          measurements: {
            statusSamples: 4,
            failedSamples: 1,
            failureRate: 0.25,
            latencySamples: 3,
            avgLatencyMs: 120,
            firstTokenSamples: 2,
            avgFirstTokenMs: 0,
            generationSpeedSamples: 0,
            avgGenerationTokensPerSecond: null,
          },
        }
      : normal(command, args),
  );
  const view = render(<UsageRequests {...props} />);
  const metrics = await screen.findByRole("region", { name: "请求测量" });
  expect(metrics).toHaveTextContent("25%");
  expect(metrics).toHaveTextContent("4 / 45");
  expect(metrics).toHaveTextContent("120 ms");
  expect(metrics).toHaveTextContent("3 / 45");
  expect(metrics).toHaveTextContent("0 ms");
  expect(metrics).toHaveTextContent("2 / 45");
  expect(metrics).toHaveTextContent("0 / 45");
  expect(metrics).toHaveTextContent("暂无可用样本");
  const trend = screen.getByRole("region", { name: "请求趋势" });
  expect(
    within(trend).getByRole("button", { name: /540 Tokens/ }),
  ).toBeVisible();
  await userEvent.click(screen.getByRole("button", { name: "下一页" }));
  await screen.findByText("第 2 / 3 页");
  view.rerender(<UsageRequests {...props} updateVersion={1} />);
  expect(screen.getByRole("region", { name: "请求测量" })).toHaveTextContent(
    "4 / 45",
  );
  expect(
    within(screen.getByRole("region", { name: "请求趋势" })).getByRole(
      "button",
      { name: /540 Tokens/ },
    ),
  ).toBeVisible();
});

it("explains older-backend absence instead of manufacturing zero-sample statistics", async () => {
  render(<UsageRequests {...props} />);
  await screen.findByRole("button", { name: /model/ });
  expect(screen.getByText("当前后端未提供请求趋势和测量摘要。")).toBeVisible();
  expect(screen.queryByRole("region", { name: "请求测量" })).toBeNull();
});

it("honors the same explicit first-token measurement provenance in request detail", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "get_request_detail"
      ? Promise.resolve({
          ...row,
          firstTokenMs: 999,
          observedFirstTokenMs: null,
          receipt: receipt(),
        })
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  await userEvent.click(await screen.findByRole("button", { name: /model/ }));
  const dialog = await screen.findByRole("dialog");
  expect(
    await within(dialog).findByText("原始记录未提供请求状态"),
  ).toBeVisible();
  expect(
    within(dialog).getByText(/原始记录未提供：.*首 Token 用时/),
  ).toBeVisible();
  expect(within(dialog).queryByText("999 ms")).toBeNull();
});

it("does not reinterpret an older proxy first-event column as observed first-token timing", async () => {
  const normal = mocks.invoke.getMockImplementation()!;
  mocks.invoke.mockImplementation((command, args) =>
    command === "get_request_detail"
      ? Promise.resolve({
          ...row,
          dataSource: "proxy",
          firstTokenMs: 999,
          receipt: receipt(),
        })
      : normal(command, args),
  );
  render(<UsageRequests {...props} />);
  await userEvent.click(await screen.findByRole("button", { name: /model/ }));
  const dialog = await screen.findByRole("dialog");
  expect(
    await within(dialog).findByText("原始记录未提供请求状态"),
  ).toBeVisible();
  expect(
    within(dialog).getByText(/原始记录未提供：.*首 Token 用时/),
  ).toBeVisible();
  expect(within(dialog).queryByText("999 ms")).toBeNull();
});
