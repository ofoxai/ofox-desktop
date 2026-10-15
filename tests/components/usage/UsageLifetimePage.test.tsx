import { useEffect } from "react";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { ShellProvider, useShell } from "@/components/shell/ShellContext";
import { UsagePage } from "@/components/usage-overview/UsagePage";
import type { OfoxAccount } from "@/hooks/useOfoxAccount";
import { queryClient } from "@/lib/query/queryClient";
import { sessionAnalyticsFixture } from "./sessionAnalyticsFixture";
import { lifetimeFixture } from "./lifetimeFixture";

const mocks = vi.hoisted(() => ({
  invoke: vi.fn(),
  supported: undefined as boolean | undefined,
}));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));
vi.mock("@/lib/api", () => ({ settingsApi: { openExternal: vi.fn() } }));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io" }),
}));
function Activate() {
  const shell = useShell();
  useEffect(() => shell.setActive("usage"), [shell.setActive]);
  return null;
}
function Page({ bound }: { bound: boolean }) {
  return (
    <ShellProvider openSettings={() => undefined}>
      <Activate />
      <UsagePage
        account={{ user: null } as OfoxAccount}
        boundTools={bound ? ["hermes"] : []}
      />
    </ShellProvider>
  );
}
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  queryClient.clear();
  localStorage.clear();
  mocks.supported = undefined;
  mocks.invoke
    .mockReset()
    .mockImplementation(async (command: string, args: any) => {
      if (command === "get_usage_capabilities")
        return {
          queryVersion: 1,
          requestSnapshots: false,
          csv: false,
          projectFilter: false,
          sources: [
            {
              toolId: "hermes",
              sourceIds: [],
              usageSupported: false,
              sessionAnalyticsSupported: false,
              projectFilterSupported: false,
              lifetimeUsageSupported: mocks.supported,
              reason: "usageReaderUnavailable",
            },
          ],
        };
      if (command === "get_usage_overview") {
        const fixture = sessionAnalyticsFixture();
        return {
          ...fixture,
          appliedQuery: args.query,
          period: "30d",
          start: args.query.startMs,
          end: args.query.endMs,
          granularity: "day",
          totals: { ...fixture.totals, requests: 3, costUsd: 0 },
          series: [],
          byTool: [],
          byModel: [],
          unsupportedTools: ["hermes"],
          lastSyncedAt: null,
        };
      }
      if (command === "get_usage_source_summary") {
        const data = lifetimeFixture();
        data.pricingCoverage.apex = "ofox.io";
        return data;
      }
      if (command === "sync_session_usage" || command === "refresh_local_usage")
        return {};
      throw new Error("Unexpected fixture command");
    });
});

it.each([
  [false, true],
  [true, false],
  [true, undefined],
] as const)(
  "gates the mounted lifetime entry with binding=%s and capability=%s",
  async (bound, supported) => {
    mocks.supported = supported;
    render(<Page bound={bound} />);
    await waitFor(() =>
      expect(mocks.invoke).toHaveBeenCalledWith("get_usage_capabilities"),
    );
    expect(
      screen.queryByRole("button", { name: "Hermes 累计用量" }),
    ).toBeNull();
    expect(
      mocks.invoke.mock.calls.some(
        ([name]) => name === "get_usage_source_summary",
      ),
    ).toBe(false);
  },
);
it("mounts the enabled bound-Hermes entry and reads only after explicit open", async () => {
  mocks.supported = true;
  render(<Page bound />);
  const entry = await screen.findByRole("button", { name: "Hermes 累计用量" });
  expect(
    mocks.invoke.mock.calls.some(
      ([name]) => name === "get_usage_source_summary",
    ),
  ).toBe(false);
  await userEvent.click(entry);
  expect(await screen.findByText("native-model")).toBeVisible();
  expect(mocks.invoke).toHaveBeenCalledWith("get_usage_source_summary", {
    toolId: "hermes",
  });
});
