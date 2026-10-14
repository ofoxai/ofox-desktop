import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
beforeEach(() => i18n.addResourceBundle("zh", "translation", zh, true, true));
import { render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import SetupCompletePage from "@/components/onboarding/SetupCompletePage";

const platform = vi.hoisted(() => ({ mac: true }));

vi.mock("@/lib/platform", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/platform")>()),
  isMac: () => platform.mac,
}));

describe("SetupCompletePage tray hint", () => {
  it("points to the menu bar on macOS", () => {
    platform.mac = true;
    render(
      <SetupCompletePage
        boundTools={["pi", "codex"]}
        onOpenConsole={() => {}}
      />,
    );
    expect(screen.getByText(/菜单栏的/)).toBeVisible();
  });

  it("points to the taskbar notification area elsewhere", () => {
    platform.mac = false;
    render(
      <SetupCompletePage
        boundTools={["pi", "codex"]}
        onOpenConsole={() => {}}
      />,
    );
    expect(screen.getByText(/任务栏通知区域的/)).toBeVisible();
    expect(screen.queryByText(/菜单栏/)).toBeNull();
  });
});

it("lists the tools actually bound without a model-count or unsupported-tool promise", () => {
  render(
    <SetupCompletePage boundTools={["pi", "codex"]} onOpenConsole={() => {}} />,
  );
  expect(screen.getByText("Pi")).toBeVisible();
  expect(screen.getByText("Codex")).toBeVisible();
  expect(screen.queryByText(/Cursor|100\+/)).toBeNull();
});
