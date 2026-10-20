import { describe, expect, it } from "vitest";
import { cn } from "@/lib/utils";

describe("cn knows the design tokens", () => {
  it("keeps a token font size beside a token text colour", () => {
    expect(cn("text-ctl", "text-ink-ctl")).toBe("text-ctl text-ink-ctl");
  });

  it("lets a later token font size replace an earlier one", () => {
    expect(cn("text-sm", "text-sub")).toBe("text-sub");
  });

  it("lets a token radius replace a stock one", () => {
    expect(cn("rounded-md", "rounded-ctl")).toBe("rounded-ctl");
  });

  it("lets a token shadow replace a stock one", () => {
    expect(cn("shadow-sm", "shadow-pop")).toBe("shadow-pop");
  });
});
