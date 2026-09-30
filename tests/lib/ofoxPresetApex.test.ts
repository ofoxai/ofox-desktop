import { describe, expect, it } from "vitest";
import { providerPresets } from "@/config/claudeProviderPresets";
import { codexProviderPresets } from "@/config/codexProviderPresets";
import { geminiProviderPresets } from "@/config/geminiProviderPresets";
import { opencodeProviderPresets } from "@/config/opencodeProviderPresets";
import { openclawProviderPresets } from "@/config/openclawProviderPresets";
import { hermesProviderPresets } from "@/config/hermesProviderPresets";
import { resolveOfoxPreset } from "@/lib/ofoxUrls";

const ofoxPresets = [
  ["Claude Code", providerPresets],
  ["Codex", codexProviderPresets],
  ["Gemini", geminiProviderPresets],
  ["OpenCode", opencodeProviderPresets],
  ["OpenClaw", openclawProviderPresets],
  ["Hermes", hermesProviderPresets],
] as const;

describe("OFox agent presets", () => {
  it.each(ofoxPresets)("resolves %s to the selected apex", (_name, presets) => {
    const preset = presets.find((item) => item.providerType === "ofox");
    expect(preset).toBeDefined();

    const resolved = JSON.stringify(resolveOfoxPreset(preset, "ofox.io"));
    expect(resolved).toContain("https://api.ofox.io/");
    expect(resolved).toContain("https://app.ofox.io/");
    expect(resolved).not.toContain("ofox.ai");

    const overseas = JSON.stringify(resolveOfoxPreset(preset, "ofox.ai"));
    expect(overseas).toContain("https://api.ofox.ai/");
    expect(JSON.stringify(preset)).toContain("https://api.ofox.ai/");
  });

  it("leaves unrelated URLs and values unchanged", () => {
    const preset = {
      url: "https://api.ofox.ai/v1",
      other: "https://example.com/ofox.ai",
      nested: ["https://ofox.ai/docs", 3, false, null],
    };
    expect(resolveOfoxPreset(preset, "ofox.io")).toEqual({
      url: "https://api.ofox.io/v1",
      other: "https://example.com/ofox.ai",
      nested: ["https://ofox.io/docs", 3, false, null],
    });
  });
});
