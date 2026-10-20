import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { UsageSourceAlert } from "@/components/usage-overview/UsageSourceAlert";
import type {
  UsagePricingCoverage,
  UsageSourceHealth,
} from "@/types/usageQuery";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

const source = (
  toolId: string,
  status: UsageSourceHealth["status"],
): UsageSourceHealth => ({
  sourceId: `${toolId}_session`,
  toolId,
  status,
  lastAttemptAtMs: 2000,
  lastSuccessAtMs: 1000,
  errorCode: "private source path",
});

const pricing = (
  overrides: Partial<UsagePricingCoverage> = {},
): UsagePricingCoverage => ({
  policy: "current-ofox-base-v1",
  apex: "ofox.io",
  catalogState: "ready",
  catalogVersion: null,
  catalogUpdatedAtMs: null,
  stale: false,
  unpricedModelIds: [],
  unpricedRecordCount: 0,
  additionalUnitsUnsupported: false,
  ...overrides,
});

it("renders nothing when every source read and every model is priced", () => {
  const { container } = render(
    <UsageSourceAlert
      sources={[source("claude", "ok"), source("opencode", "partial")]}
      pricing={pricing({ stale: true, additionalUnitsUnsupported: true })}
    />,
  );
  expect(container).toBeEmptyDOMElement();
});

it("names failed tools once, hides error details and retries", async () => {
  const retry = vi.fn();
  render(
    <UsageSourceAlert
      sources={[
        source("opencode", "failed"),
        source("opencode", "failed"),
        source("hermes", "failed"),
        source("claude", "unknown"),
      ]}
      onRetry={retry}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "OpenCode · Hermes 读取失败，显示的是上次的数据",
  );
  expect(screen.queryByText(/private source path/)).toBeNull();
  expect(screen.queryByText(/Claude/)).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "重试" }));
  expect(retry).toHaveBeenCalledOnce();
});

it("counts unpriced models, or says the price catalog is unavailable", () => {
  const view = render(
    <UsageSourceAlert pricing={pricing({ unpricedModelIds: ["a", "b"] })} />,
  );
  expect(screen.getByRole("status")).toHaveTextContent("2 个模型未计价");
  expect(screen.queryByRole("button")).toBeNull();
  view.rerender(
    <UsageSourceAlert
      pricing={pricing({ catalogState: "invalid", unpricedModelIds: ["a"] })}
    />,
  );
  expect(screen.getByRole("status")).toHaveTextContent(
    "价格目录不可用，费用暂无法估算",
  );
});
