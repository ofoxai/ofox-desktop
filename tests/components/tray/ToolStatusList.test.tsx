import { render, waitFor } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { QueryClientProvider } from "@tanstack/react-query";
import { queryClient } from "@/lib/query/queryClient";
import ToolStatusList from "@/components/tray/ToolStatusList";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  queryClient.clear();
  mocks.invoke.mockReset();
  mocks.invoke.mockResolvedValue({ tools: [] });
});

it("asks only for the tools it can show, so one unknown id can't fail them all", async () => {
  render(
    <QueryClientProvider client={queryClient}>
      <ToolStatusList boundTools={["hermes", "not-a-tool", "claude"]} />
    </QueryClientProvider>,
  );
  await waitFor(() =>
    expect(mocks.invoke).toHaveBeenCalledWith("model_selection_snapshot", {
      tools: ["claude", "hermes"],
    }),
  );
});
