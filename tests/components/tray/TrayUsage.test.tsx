import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import TrayUsage from "@/components/tray/TrayUsage";
import * as usageOverview from "@/lib/api/usageOverview";
import type { UsageOverview } from "@/lib/api/usageOverview";
import * as apexApi from "@/lib/api/ofoxApex";
import { queryClient } from "@/lib/query/queryClient";

function startOfDay(daysBack = 0) {
  const day = new Date();
  day.setHours(0, 0, 0, 0);
  day.setDate(day.getDate() - daysBack);
  return day.getTime();
}

beforeEach(() => {
  queryClient.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.spyOn(apexApi, "ofoxGetApexState").mockResolvedValue({
    apex: "ofox.io",
    pinned: true,
    detected: true,
  });
  vi.spyOn(usageOverview, "getUsageOverview").mockResolvedValue({
    byTool: [],
  } as unknown as UsageOverview);
});
afterEach(() => vi.restoreAllMocks());

describe("TrayUsage", () => {
  it("reads today through the same query as the usage page", async () => {
    render(<TrayUsage />);

    await waitFor(() =>
      expect(usageOverview.getUsageOverview).toHaveBeenCalled(),
    );
    const [period, query] = vi.mocked(usageOverview.getUsageOverview).mock
      .calls[0];
    expect(period).toBe("today");
    expect(query).toMatchObject({ version: 1, startMs: startOfDay() });
  });

  it("reads the last seven days through the same query", async () => {
    render(<TrayUsage />);
    fireEvent.click(await screen.findByRole("radio", { name: "7 天" }));

    await waitFor(() =>
      expect(usageOverview.getUsageOverview).toHaveBeenCalledWith(
        "7d",
        expect.objectContaining({ version: 1, startMs: startOfDay(6) }),
        "ofox.io",
      ),
    );
  });
});
