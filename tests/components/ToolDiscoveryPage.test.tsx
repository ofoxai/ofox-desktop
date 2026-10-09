import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import * as tauri from "@tauri-apps/api/core";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import ToolDiscoveryPage from "@/components/onboarding/ToolDiscoveryPage";
import { TOOL_ORDER } from "@/config/toolMeta";
import type { ToolInstallationInfo } from "@/lib/api/toolUpdates";

vi.mock("@/hooks/useToolInstall", () => ({
  useToolInstall: () => ({
    installing: new Set(),
    install: vi.fn(),
    error: null,
    progress: {},
  }),
}));
vi.mock("@/hooks/useToolInstallCapabilities", () => ({
  useToolInstallCapabilities: () => [],
}));
beforeAll(() => i18n.addResourceBundle("zh", "translation", zh, true, true));
afterEach(() => vi.restoreAllMocks());

describe("ToolDiscoveryPage scan lifecycle", () => {
  it("does not let an older retry override the latest scan and the user's selection", async () => {
    const current = TOOL_ORDER.map(
      (name): ToolInstallationInfo => ({
        name,
        version: ["claude", "codex"].includes(name) ? "1.0" : null,
        error: name === "gemini" ? "detection error" : null,
        installationStatus: ["claude", "codex"].includes(name)
          ? "installed"
          : name === "gemini"
            ? "unknown"
            : "notInstalled",
      }),
    );
    let finishFirst!: (tools: ToolInstallationInfo[]) => void;
    const firstRetry = new Promise<ToolInstallationInfo[]>((resolve) => {
      finishFirst = resolve;
    });
    vi.spyOn(tauri, "invoke")
      .mockResolvedValueOnce(current)
      .mockReturnValueOnce(firstRetry)
      .mockResolvedValueOnce(current);
    const bind = vi.fn();
    render(<ToolDiscoveryPage onBind={bind} />);
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "开始绑定（2）→" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "开始绑定（2）→" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByText("Codex").closest("button")!);
    await act(async () => finishFirst(current));
    const confirm = screen.getByRole("button", { name: "开始绑定（1）→" });
    expect(confirm).toBeEnabled();
    fireEvent.click(confirm);
    expect(bind).toHaveBeenCalledWith(["claude"]);
  });
});
