import { describe, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";

import {
  AUTO_SELECTION,
  OfoxApexSwitch,
  planSelection,
} from "@/components/OfoxApexSwitch";

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
});
