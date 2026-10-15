import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageLifetimeEntry } from "@/components/usage-overview/UsageLifetimeEntry";
import type { UsageCapabilities } from "@/types/usageQuery";
import type { UsageLifetimeSummary } from "@/types/usageLifetime";
import { lifetimeFixture } from "./lifetimeFixture";

const read = vi.hoisted(() => vi.fn());
vi.mock("@/lib/api/usageLifetime", () => ({ getUsageSourceSummary: read }));
const capability = (supported?: boolean): UsageCapabilities => ({
  queryVersion: 1,
  requestSnapshots: true,
  csv: true,
  projectFilter: false,
  sources: [
    {
      toolId: "hermes",
      sourceIds: [],
      usageSupported: false,
      sessionAnalyticsSupported: false,
      projectFilterSupported: false,
      lifetimeUsageSupported: supported,
      reason: "usageReaderUnavailable",
    },
  ],
});
const props = {
  boundTools: ["hermes"],
  capabilities: capability(true),
  apex: "ofox.ai",
  active: true,
};
const trigger = () => screen.getByRole("button", { name: "Hermes 累计用量" });
async function open() {
  await userEvent.click(trigger());
  return screen.findByRole("dialog");
}
beforeEach(() => {
  read.mockReset().mockResolvedValue(lifetimeFixture());
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

it.each([undefined, false])(
  "does not advertise lifetime data without an explicit backend capability (%s)",
  (supported) => {
    render(
      <UsageLifetimeEntry {...props} capabilities={capability(supported)} />,
    );
    expect(screen.queryByRole("button")).toBeNull();
    expect(read).not.toHaveBeenCalled();
  },
);
it("does not read a tool that is not bound", () => {
  render(<UsageLifetimeEntry {...props} boundTools={["codex"]} />);
  expect(screen.queryByRole("button")).toBeNull();
  expect(read).not.toHaveBeenCalled();
});
it("loads only on explicit open and explains its independent cumulative scope", async () => {
  render(<UsageLifetimeEntry {...props} />);
  expect(read).not.toHaveBeenCalled();
  const dialog = await open();
  await within(dialog).findByText("native-model");
  expect(read).toHaveBeenCalledTimes(1);
  expect(read).toHaveBeenCalledWith("hermes", "ofox.ai");
  expect(
    within(dialog).getByText(/不受时间.*不参与每日趋势.*请求列表/),
  ).toBeVisible();
  expect(within(dialog).queryByRole("combobox")).toBeNull();
  expect(within(dialog).getByText("计数覆盖：完整")).toBeVisible();
  expect(within(dialog).getByText("模型归属：部分")).toBeVisible();
});
it("shows fresh input plus output and keeps unknown cache distinct from zero", async () => {
  const data = lifetimeFixture();
  data.totals.cacheReadTokens = null;
  data.totals.amountKind = "lowerBound";
  read.mockResolvedValue(data);
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  await within(dialog).findByText("native-model");
  expect(
    within(within(dialog).getByText("Tokens").parentElement!).getByText("≥50"),
  ).toBeVisible();
  expect(
    within(within(dialog).getByText("缓存读取").parentElement!).getByText("—"),
  ).toBeVisible();
  expect(
    within(dialog).getByText(/原生累计 API 调用计数.*不是可逐条/),
  ).toBeVisible();
});
it("keeps source readiness, model attribution and price coverage separate", async () => {
  const data = lifetimeFixture();
  data.estimate.availability = "unknown";
  data.estimate.knownCostUsd = null;
  data.pricingCoverage.catalogState = "invalid";
  read.mockResolvedValue(data);
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  await within(dialog).findByText("native-model");
  expect(within(dialog).getByText("计数覆盖：完整")).toBeVisible();
  expect(within(dialog).getByText("模型归属：部分")).toBeVisible();
  expect(within(dialog).getByText("价格估算：未知")).toBeVisible();
  expect(within(dialog).getByText(/价格目录不可用/)).toBeVisible();
});
it("shows persisted last success as stale when the native source is missing", async () => {
  const data = lifetimeFixture();
  data.sourceState = "missing";
  data.stale = true;
  read.mockResolvedValue(data);
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  await within(dialog).findByText("native-model");
  expect(within(dialog).getByText(/源文件缺失.*保留上次/)).toBeVisible();
  expect(within(dialog).getByText(/^上次稳定读取/)).toBeVisible();
});
it("does not call an absent source corrupt or show fabricated zero counters", async () => {
  const data = lifetimeFixture();
  data.sourceState = "missing";
  data.snapshotVersion = null;
  data.lastSuccessAtMs = null;
  data.sessionRecords = null;
  data.byModel = [];
  data.totals = {
    inputTokens: null,
    outputTokens: null,
    cacheReadTokens: null,
    cacheWriteTokens: null,
    totalTokens: null,
    apiCallCount: null,
    amountKind: "unknown",
  };
  read.mockResolvedValue(data);
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  expect(await within(dialog).findByText(/尚无可读取的原生累计/)).toBeVisible();
  expect(within(dialog).queryByText("Tokens")).toBeNull();
});
it("keeps an unattributed residual explicit and bounds model rows to 50", async () => {
  const data = lifetimeFixture();
  data.byModel = Array.from({ length: 120 }, (_, n) => ({
    ...data.byModel[0],
    modelId: n ? `model-${n}` : null,
    category: n ? "auxiliary" : "unattributed",
  }));
  read.mockResolvedValue(data);
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  const list = await within(dialog).findByRole("list", { name: "模型与用途" });
  expect(within(list).getAllByRole("listitem")).toHaveLength(50);
  expect(within(list).getByText("无法归属模型的余量")).toBeVisible();
  await userEvent.click(
    within(dialog).getByRole("button", { name: "显示更多（剩余 70）" }),
  );
  expect(within(list).getAllByRole("listitem")).toHaveLength(100);
});
it("retains the current summary if an explicit refresh fails and scrubs errors", async () => {
  render(<UsageLifetimeEntry {...props} />);
  const dialog = await open();
  await within(dialog).findByText("native-model");
  read.mockRejectedValueOnce(new Error("/private/native/path token=SECRET"));
  await userEvent.click(
    within(dialog).getByRole("button", { name: "重新读取" }),
  );
  expect(await within(dialog).findByRole("alert")).toHaveTextContent(
    "累计用量读取失败",
  );
  expect(within(dialog).getByText("native-model")).toBeVisible();
  expect(dialog).not.toHaveTextContent("SECRET");
});
it("closes on Escape and returns focus to the cumulative entry", async () => {
  render(<UsageLifetimeEntry {...props} />);
  const button = trigger();
  button.focus();
  await userEvent.keyboard("{Enter}");
  await screen.findByText("native-model");
  await userEvent.keyboard("{Escape}");
  await waitFor(() => expect(button).toHaveFocus());
});
it("rejects an old in-flight completion after apex changes", async () => {
  let finish!: (data: UsageLifetimeSummary) => void;
  read.mockReturnValueOnce(
    new Promise<UsageLifetimeSummary>((resolve) => {
      finish = resolve;
    }),
  );
  const view = render(<UsageLifetimeEntry {...props} />);
  await open();
  view.rerender(<UsageLifetimeEntry {...props} apex="ofox.io" />);
  expect(screen.queryByRole("dialog")).toBeNull();
  await act(async () => {
    finish(lifetimeFixture());
  });
  expect(screen.queryByText("native-model")).toBeNull();
  const current = lifetimeFixture();
  current.byModel[0].modelId = "current-model";
  current.pricingCoverage.apex = "ofox.io";
  read.mockResolvedValueOnce(current);
  const dialog = await open();
  expect(await within(dialog).findByText("current-model")).toBeVisible();
  expect(read).toHaveBeenLastCalledWith("hermes", "ofox.io");
});
it("ignores completions from a hidden page even if it becomes active again", async () => {
  let finish!: (data: UsageLifetimeSummary) => void;
  read.mockReturnValueOnce(
    new Promise<UsageLifetimeSummary>((resolve) => {
      finish = resolve;
    }),
  );
  const view = render(<UsageLifetimeEntry {...props} />);
  await open();
  view.rerender(<UsageLifetimeEntry {...props} active={false} />);
  view.rerender(<UsageLifetimeEntry {...props} />);
  await act(async () => {
    finish(lifetimeFixture());
  });
  expect(screen.queryByRole("dialog")).toBeNull();
  expect(screen.queryByText("native-model")).toBeNull();
});
