import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  UsageFreshness,
  UsagePriceNotice,
} from "@/components/usage-overview/UsageFreshness";
import type { UsageSourceHealth } from "@/types/usageQuery";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

const source = (
  observation: Record<string, unknown> = {},
): UsageSourceHealth => ({
  sourceId: "gemini_session",
  toolId: "gemini",
  status: "ok",
  lastAttemptAtMs: 2000,
  lastSuccessAtMs: 2000,
  errorCode: null,
  ...observation,
});

it("shows known zero scan counts and labels the retained detail interval as incomplete history", () => {
  render(
    <UsageFreshness
      sources={[
        source({
          lastScan: { filesScanned: 2, imported: 0, skipped: 0 },
          retainedDetailRange: { startMs: 0, endMs: 3000 },
        }),
      ]}
    />,
  );
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText(/最近扫描.*2.*导入.*0.*跳过.*0/)).toBeVisible();
  const range = screen.getByText(/已保留明细.*不代表完整历史/);
  expect(range).toHaveTextContent(new Date(0).toLocaleString("zh"));
  expect(range).toHaveTextContent(new Date(2999).toLocaleString("zh"));
});

it("keeps failed or pending scan observations unknown without showing old counts as zero", () => {
  render(
    <UsageFreshness
      sources={[
        source({ status: "failed", lastScan: null, retainedDetailRange: null }),
      ]}
    />,
  );
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText("最近扫描结果未知")).toBeVisible();
  expect(screen.getByText("已保留明细范围未记录")).toBeVisible();
  expect(screen.queryByText(/导入 0/)).toBeNull();
});

it("treats an older backend without observation fields as unknown", () => {
  render(<UsageFreshness sources={[source()]} />);
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText("最近扫描结果未知")).toBeVisible();
  expect(screen.getByText("已保留明细范围未记录")).toBeVisible();
});

it("separates source failures and price coverage from successful local and account times", () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  render(
    <UsageFreshness
      localUpdatedAt={null}
      accountUpdatedAt={1000}
      sources={[
        {
          sourceId: "claude_session",
          toolId: "claude",
          status: "failed",
          lastAttemptAtMs: 2000,
          lastSuccessAtMs: 500,
          errorCode: "private path must never render",
        },
      ]}
      pricing={{
        policy: "current-ofox-base-v1",
        apex: "ofox.io",
        catalogState: "invalid",
        catalogVersion: null,
        catalogUpdatedAtMs: null,
        stale: true,
        unpricedModelIds: ["unknown"],
        unpricedRecordCount: 2,
        additionalUnitsUnsupported: true,
      }}
    />,
  );
  expect(screen.getByText(/本机同步.*未记录/)).not.toBeVisible();
  expect(screen.getByText(/账户余额更新/)).not.toBeVisible();
  expect(
    within(screen.getByRole("status")).getByText(/Claude Code.*失败/),
  ).toBeVisible();
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText(/本机同步.*未记录/)).toBeVisible();
  expect(screen.getByText(/账户余额更新/)).toBeVisible();
  expect(screen.getByText(/价格目录不可用/)).toBeVisible();
  expect(screen.queryByText(/private path/)).toBeNull();
});

it("keeps routine sync times and the full catalog version collapsed until data status is opened", () => {
  const version = "catalog-" + "a".repeat(64);
  render(
    <UsageFreshness
      localUpdatedAt={1000}
      accountUpdatedAt={2000}
      sources={[source()]}
      pricing={{
        policy: "current-ofox-base-v1",
        apex: "ofox.io",
        catalogState: "ready",
        catalogVersion: version,
        catalogUpdatedAtMs: 3000,
        stale: false,
        unpricedModelIds: [],
        unpricedRecordCount: 0,
        additionalUnitsUnsupported: false,
      }}
    />,
  );
  expect(screen.getByText(/本机同步/)).not.toBeVisible();
  expect(screen.getByText(/账户余额更新/)).not.toBeVisible();
  expect(screen.getByText(new RegExp(version))).not.toBeVisible();
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText(/本机同步/)).toBeVisible();
  expect(screen.getByText(/账户余额更新/)).toBeVisible();
  expect(screen.getByText(new RegExp(version))).toBeVisible();
  expect(
    screen.getAllByText("费用为按 Ofox 价格的估算，以账单为准"),
  ).toHaveLength(1);
});

it("keeps failed source and price coverage warnings visible while routine metadata is collapsed", () => {
  render(
    <UsageFreshness
      localUpdatedAt={1000}
      sources={[source({ status: "failed", errorCode: "private source path" })]}
      pricing={{
        policy: "current-ofox-base-v1",
        apex: "ofox.io",
        catalogState: "invalid",
        catalogVersion: null,
        catalogUpdatedAtMs: null,
        stale: true,
        unpricedModelIds: ["missing-model"],
        unpricedRecordCount: 1,
        additionalUnitsUnsupported: false,
      }}
    />,
  );
  expect(
    within(screen.getByRole("status")).getByText(/Gemini.*失败/),
  ).toBeVisible();
  expect(screen.getByText(/价格目录不可用/)).toBeVisible();
  expect(screen.getByText(/1 个模型缺少基础 Token 价格/)).toBeVisible();
  expect(screen.getByText(/本机同步/)).not.toBeVisible();
  expect(screen.queryByText(/private source path/)).toBeNull();
});

it("also folds request price metadata while keeping stale price warnings visible", () => {
  render(
    <UsagePriceNotice
      pricing={{
        policy: "current-ofox-base-v1",
        apex: "ofox.ai",
        catalogState: "ready",
        catalogVersion: "request-catalog",
        catalogUpdatedAtMs: 1000,
        stale: true,
        unpricedModelIds: [],
        unpricedRecordCount: 0,
        additionalUnitsUnsupported: false,
      }}
    />,
  );
  expect(screen.getByText(/价格目录较旧/)).toBeVisible();
  expect(screen.getByText(/价格目录更新/)).not.toBeVisible();
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText(/价格目录更新/)).toBeVisible();
});

it("summarizes repeated tool failures once while keeping each source in the details", () => {
  render(
    <UsageFreshness
      sources={[
        source({
          sourceId: "claude_primary",
          toolId: "claude",
          status: "failed",
        }),
        source({
          sourceId: "claude_archive",
          toolId: "claude",
          status: "failed",
        }),
      ]}
    />,
  );
  expect(
    within(screen.getByRole("status")).getAllByText(/Claude Code.*失败/),
  ).toHaveLength(1);
  fireEvent.click(screen.getByText("数据状态"));
  expect(screen.getByText(/claude_primary/)).toBeVisible();
  expect(screen.getByText(/claude_archive/)).toBeVisible();
});
