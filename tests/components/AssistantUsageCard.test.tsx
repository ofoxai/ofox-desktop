import { act, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantUsageCard } from "@/components/assistant/AssistantUsageCard";
import type { AssistantUsageSummary } from "@/lib/api/assistant";
const mocks = vi.hoisted(() => ({ usage: vi.fn(), listen: vi.fn() }));
vi.mock("@/lib/api/assistant", () => ({ assistantApi: mocks }));
vi.mock("@tauri-apps/api/event", () => ({
  listen: mocks.listen,
}));
const summary: AssistantUsageSummary = {
  sessionCount: 2,
  runCount: 3,
  inputTokens: 1234,
  outputTokens: 456,
  estimatedCostUsd: null,
  billedCostUsd: null,
  searchCount: 4,
  searchEstimatedCostUsd: "0.04",
};
beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  mocks.usage.mockResolvedValue(summary);
  mocks.listen.mockResolvedValue(() => undefined);
});
it("labels internal assistant estimates separately and preserves unknown billing", async () => {
  render(<AssistantUsageCard identity="fixture-owner" active />);
  expect(
    await screen.findByText(
      "Built-in assistant · Retained local conversations",
    ),
  ).toBeVisible();
  expect(screen.getByText("1,234")).toBeVisible();
  expect(
    screen.getByText(/Actual charges are not available in this local summary/),
  ).toBeVisible();
  expect(screen.getByText("$0.04")).toBeVisible();
  expect(screen.queryByText("$0.00")).not.toBeInTheDocument();
});
it("never displays a late summary from the previous account", async () => {
  let resolve!: (value: AssistantUsageSummary) => void;
  mocks.usage
    .mockReturnValueOnce(
      new Promise<AssistantUsageSummary>((done) => {
        resolve = done;
      }),
    )
    .mockResolvedValueOnce({ ...summary, inputTokens: 999 });
  const view = render(<AssistantUsageCard identity="owner-a" active />);
  await waitFor(() => expect(mocks.usage).toHaveBeenCalledTimes(1));
  view.rerender(<AssistantUsageCard identity="owner-b" active />);
  expect(await screen.findByText("999")).toBeVisible();
  await act(async () => {
    resolve(summary);
  });
  expect(screen.queryByText("1,234")).not.toBeInTheDocument();
});

it("ignores disposed event callbacks even before listener registration completes", async () => {
  const registrations: {
    name: string;
    callback: (event: { payload: { type: string } }) => void;
    resolve: (off: () => void) => void;
  }[] = [];
  mocks.listen.mockImplementation(
    (name, callback) =>
      new Promise<() => void>((resolve) =>
        registrations.push({ name, callback, resolve }),
      ),
  );
  mocks.usage
    .mockResolvedValueOnce(summary)
    .mockResolvedValue({ ...summary, inputTokens: 999 });
  const view = render(<AssistantUsageCard identity="owner-a" active />);
  await waitFor(() => expect(registrations).toHaveLength(3));
  view.rerender(<AssistantUsageCard identity="owner-b" active />);
  expect(await screen.findByText("999")).toBeVisible();
  await act(async () =>
    registrations[2].callback({ payload: { type: "run_end" } }),
  );
  expect(mocks.usage).toHaveBeenCalledTimes(2);
  view.unmount();
  await act(async () => {
    for (const registration of registrations) {
      registration.callback({ payload: { type: "run_end" } });
      registration.resolve(() => undefined);
    }
  });
  expect(mocks.usage).toHaveBeenCalledTimes(2);
});
