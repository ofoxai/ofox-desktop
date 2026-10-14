import { beforeAll, describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  AUTO_SELECTION,
  OfoxApexSwitch,
  planSelection,
} from "@/components/OfoxApexSwitch";
import { ofoxSetApex, ofoxSetApexAuto } from "@/lib/api/ofoxApex";

const apexState = vi.hoisted(() => ({
  apex: "ofox.io" as "ofox.ai" | "ofox.io",
  pinned: false,
}));

vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({
    apex: apexState.apex,
    pinned: apexState.pinned,
    isLoading: false,
    refetch: vi.fn(),
  }),
}));

vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({ isActive: false }),
}));

vi.mock("@/lib/api/ofoxApex", () => ({
  ofoxSetApex: vi.fn(),
  ofoxSetApexAuto: vi.fn(),
  OFOX_APEX_CHANGED_EVENT: "ofox-apex-changed",
}));

beforeAll(() => {
  // cmdk scrolls the highlighted option into view; jsdom has no layout.
  Element.prototype.scrollIntoView = vi.fn();
});

async function pick(option: string) {
  const user = userEvent.setup();
  await user.click(screen.getByRole("combobox", { name: "apexSwitch.label" }));
  await user.click(screen.getByRole("option", { name: option }));
}

describe("planSelection", () => {
  const auto = { apex: "ofox.io" as const, pinned: false, isActive: true };
  const pinned = { ...auto, pinned: true };

  it("does nothing when the selection matches the current state", () => {
    expect(planSelection(AUTO_SELECTION, auto)).toEqual({ kind: "noop" });
    expect(planSelection("ofox.io", pinned)).toEqual({ kind: "noop" });
  });

  it("pins the current region without clearing the session", () => {
    expect(planSelection("ofox.io", auto)).toEqual({
      kind: "pin",
      apex: "ofox.io",
    });
  });

  it("asks for confirmation before a switch only while signed in", () => {
    expect(planSelection("ofox.ai", auto)).toEqual({
      kind: "switch",
      apex: "ofox.ai",
      confirm: true,
    });
    expect(planSelection("ofox.ai", { ...auto, isActive: false })).toEqual({
      kind: "switch",
      apex: "ofox.ai",
      confirm: false,
    });
  });

  it("returns to auto-detect from a pinned region", () => {
    expect(planSelection(AUTO_SELECTION, pinned)).toEqual({
      kind: "auto",
      confirm: true,
    });
    expect(
      planSelection(AUTO_SELECTION, { ...pinned, isActive: false }),
    ).toEqual({ kind: "auto", confirm: false });
  });
});

describe("OfoxApexSwitch", () => {
  it("shows the detected region as automatic when not pinned", () => {
    apexState.apex = "ofox.io";
    apexState.pinned = false;
    render(<OfoxApexSwitch />);
    expect(screen.getByRole("combobox")).toHaveTextContent(
      "apexSwitch.autoLabel",
    );
  });

  it("shows the plain region when the user pinned it", () => {
    apexState.apex = "ofox.io";
    apexState.pinned = true;
    render(<OfoxApexSwitch />);
    expect(screen.getByRole("combobox")).toHaveTextContent("ofox.io");
    expect(screen.getByRole("combobox")).not.toHaveTextContent(
      "apexSwitch.autoLabel",
    );
  });

  it("opens the app's own menu with auto-detect and both regions, the current choice ticked", async () => {
    apexState.apex = "ofox.io";
    apexState.pinned = true;
    render(<OfoxApexSwitch />);
    const trigger = screen.getByRole("combobox", { name: "apexSwitch.label" });
    expect(trigger).toHaveAttribute("aria-haspopup", "listbox");

    await userEvent.setup().click(trigger);
    const options = within(screen.getByRole("listbox")).getAllByRole("option");
    expect(options.map((option) => option.textContent)).toEqual([
      "apexSwitch.auto",
      "ofox.ai",
      "ofox.io",
    ]);
    expect(screen.getByRole("option", { name: "ofox.io" })).toHaveAttribute(
      "data-checked",
      "true",
    );
  });

  it("switches to the region picked from the menu while signed out", async () => {
    apexState.apex = "ofox.io";
    apexState.pinned = true;
    vi.mocked(ofoxSetApex).mockResolvedValue(true);
    render(<OfoxApexSwitch />);
    await pick("ofox.ai");
    await waitFor(() => expect(ofoxSetApex).toHaveBeenCalledWith("ofox.ai"));
  });

  it("goes back to auto-detect when that is picked from a pinned region", async () => {
    apexState.apex = "ofox.ai";
    apexState.pinned = true;
    vi.mocked(ofoxSetApexAuto).mockResolvedValue({
      apex: "ofox.ai",
      pinned: false,
      detected: true,
    });
    render(<OfoxApexSwitch />);
    await pick("apexSwitch.auto");
    await waitFor(() => expect(ofoxSetApexAuto).toHaveBeenCalledTimes(1));
    expect(ofoxSetApex).not.toHaveBeenCalled();
  });
});
