import { describe, expect, it } from "vitest";

import { repairActionFor } from "@/config/toolMeta";

describe("repairActionFor", () => {
  it("reinstalls tools the bundled installer knows", () => {
    expect(repairActionFor("hermes")).toBe("install");
    expect(repairActionFor("claude")).toBe("install");
  });

  it("opens the download page for desktop apps without an installer", () => {
    expect(repairActionFor("workbuddy")).toBe("download");
  });

  it("has nothing to offer for unknown tools", () => {
    expect(repairActionFor("not-a-tool")).toBe("none");
  });
});
