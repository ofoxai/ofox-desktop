import { useState } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import type { ManagedSession, SessionUsage } from "@/lib/api/sessions";
import { SessionRow } from "@/components/sessions/SessionRow";

const usage: SessionUsage = {
  models: [
    {
      model: "known-model",
      requests: 1,
      inputTokens: 1000,
      outputTokens: 200,
      cacheReadTokens: 5000,
      cacheWriteTokens: 50,
      costUsd: 0.02,
    },
  ],
  inputTokens: 1000,
  outputTokens: 200,
  cacheReadTokens: 5000,
  cacheWriteTokens: 50,
  totalTokens: 1200,
  costUsd: 0.02,
  unpricedModels: 0,
};
const session: ManagedSession = {
  sessionId: "session-id-for-details",
  sessionKey: "verified-key",
  revision: "revision",
  providerId: "codex",
  title: "Clear session title",
  projectDir: "/example/project",
  messageCount: 33,
  sizeBytes: 11000000,
  models: ["known-model"],
  usage,
  capabilities: {
    canReadMessages: true,
    canExport: false,
    canResume: false,
    canTrash: true,
    resumeUsesSessionModel: false,
  },
};
function Row({ value }: { value: ManagedSession }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <SessionRow
      session={value}
      expanded={expanded}
      toggle={() => setExpanded(!expanded)}
      selected={false}
      select={() => undefined}
      busy={false}
      trash={() => undefined}
    />
  );
}
function show(value = session) {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <Row value={value} />
    </QueryClientProvider>,
  );
}
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});
describe("compact session metadata and honest usage states", () => {
  it("keeps the list readable and reveals file, id and token breakdown on demand", () => {
    show();
    const title = screen.getByRole("button", { name: /Clear session title/ });
    expect(within(title).getByText(/33 条消息/)).toBeVisible();
    expect(title).not.toHaveTextContent("11 MB");
    expect(title).not.toHaveTextContent("session-");
    expect(screen.queryByText(/输入 1K · 输出 200 · 缓存 5K/)).toBeNull();
    fireEvent.click(title);
    expect(screen.getByText(/session-id-for-details/)).toBeVisible();
    expect(screen.getByText(/11 MB/)).toBeVisible();
    expect(
      screen.getByText(/输入 1K · 输出 200 · 缓存 5K · 写入 50/),
    ).toBeVisible();
  });

  it("keeps aggregate token details reachable when an older backend has no model breakdown", () => {
    show({ ...session, usage: { ...usage, models: [] } });
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    expect(
      screen.getByText("输入 1K · 输出 200 · 缓存 5K · 写入 50"),
    ).toBeVisible();
  });

  it("explains absent usage instead of showing two unexplained dashes", () => {
    show({ ...session, usage: undefined });
    expect(screen.getByText("用量不可用")).toBeVisible();
    expect(screen.queryAllByText("—")).toHaveLength(0);
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    expect(
      screen.getByText("当前会话没有可核验的用量记录，无法估算费用。"),
    ).toBeVisible();
  });

  it("distinguishes missing model prices from missing tokens and a known zero cost", () => {
    const rendered = show({
      ...session,
      usage: {
        ...usage,
        models: [{ ...usage.models[0], costUsd: null }],
        costUsd: 0,
        unpricedModels: 1,
      },
    });
    expect(screen.getByText("1.2K")).toBeVisible();
    expect(screen.getByText("未计价")).toBeVisible();
    expect(screen.queryByText("≈$0.000")).toBeNull();
    rendered.unmount();
    show({
      ...session,
      usage: {
        ...usage,
        costUsd: 0,
        models: [{ ...usage.models[0], costUsd: 0 }],
      },
    });
    expect(screen.getByText("≈$0.000")).toBeVisible();
    expect(screen.queryByText("未计价")).toBeNull();
  });

  it("does not present an ambiguous root ledger total as this thread's own cost", () => {
    show({
      ...session,
      usageDisplay: {
        state: "unavailable",
        basis: "currentOfoxEstimate",
        scope: "nativeThread",
        reasons: ["ambiguousThreadHistory"],
        estimate: null,
        parentSessionId: "parent",
        billingSessionId: "parent",
      },
    });
    expect(screen.getByText("用量不可用")).toBeVisible();
    expect(screen.queryByText("1.2K")).toBeNull();
    expect(screen.queryByText("≈$0.020")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    expect(
      screen.getByText(
        "此会话包含无法区分归属的复制历史，暂不展示用量和费用。",
      ),
    ).toBeVisible();
  });

  it("uses the current thread estimate and explains its own-thread scope", () => {
    show({
      ...session,
      usageDisplay: {
        state: "available",
        basis: "currentOfoxEstimate",
        scope: "nativeThread",
        reasons: [],
        estimate: {
          knownCostUsd: "1.23",
          availability: "known",
          unpricedModelIds: [],
          unpricedRecordCount: 0,
          additionalUnitsUnsupported: false,
        },
        parentSessionId: "parent",
        billingSessionId: "parent",
      },
    });
    expect(screen.getByText("≈$1.23")).toBeVisible();
    expect(screen.queryByText("≈$0.020")).toBeNull();
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    expect(
      screen.getByText(
        "仅此会话的可核验用量，子会话另列；费用按当前 OFox 价格估算，不代表账户实际扣费。",
      ),
    ).toBeVisible();
  });

  it("keeps a model's partial pricing explicit in the expanded breakdown", () => {
    const estimate = {
      knownCostUsd: "0.02",
      availability: "partial" as const,
      unpricedModelIds: ["known-model"],
      unpricedRecordCount: 1,
      additionalUnitsUnsupported: false,
    };
    show({
      ...session,
      usageDisplay: {
        state: "available",
        basis: "currentOfoxEstimate",
        scope: "nativeThread",
        reasons: [],
        parentSessionId: null,
        billingSessionId: null,
        estimate,
        modelEstimates: { "known-model": estimate },
      },
    });
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    const modelRow = screen.getByText("known-model", {
      selector: "code",
    }).parentElement!;
    expect(within(modelRow).getByText("≈$0.020+")).toBeVisible();
  });

  it("retains the partial-cost plus sign and an accessible explanation", () => {
    show({
      ...session,
      usage: {
        ...usage,
        models: [
          usage.models[0],
          { ...usage.models[0], model: "unknown", costUsd: null },
        ],
        unpricedModels: 1,
      },
    });
    expect(screen.getByText("≈$0.020+")).toBeVisible();
    fireEvent.click(
      screen.getByRole("button", { name: /Clear session title/ }),
    );
    expect(
      screen.getByText("仅包含有价格的模型，部分费用尚未计入。"),
    ).toBeVisible();
  });
});
