import { afterEach, describe, expect, it, vi } from "vitest";
import { platformClass } from "@/lib/platform";

function asBrowser(userAgent: string, platform = "") {
  vi.spyOn(navigator, "userAgent", "get").mockReturnValue(userAgent);
  vi.spyOn(navigator, "platform", "get").mockReturnValue(platform);
}

describe("platformClass", () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it("names the Mac", () => {
    asBrowser(
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15",
      "MacIntel",
    );
    expect(platformClass()).toBe("mac");
  });

  it("names Windows", () => {
    asBrowser(
      "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 Edg/130.0",
      "Win32",
    );
    expect(platformClass()).toBe("win");
  });

  it("treats anything else as Linux", () => {
    asBrowser("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15", "Linux");
    expect(platformClass()).toBe("linux");
  });
});
