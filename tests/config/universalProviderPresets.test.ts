import { describe, expect, it } from "vitest";
import { universalProviderPresets } from "@/config/universalProviderPresets";

describe("Universal Provider Presets - OfoxAI Integration", () => {
  it("OfoxAI should be the first preset in the list", () => {
    expect(universalProviderPresets.length).toBeGreaterThanOrEqual(1);
    expect(universalProviderPresets[0].name).toBe("OfoxAI");
  });

  it("OfoxAI preset should have correct providerType", () => {
    const ofox = universalProviderPresets[0];
    expect(ofox.providerType).toBe("ofox");
  });

  it("OfoxAI preset should enable all three apps", () => {
    const ofox = universalProviderPresets[0];
    expect(ofox.defaultApps.claude).toBe(true);
    expect(ofox.defaultApps.codex).toBe(true);
    expect(ofox.defaultApps.gemini).toBe(true);
  });

  it("OfoxAI preset should have correct branding", () => {
    const ofox = universalProviderPresets[0];
    expect(ofox.icon).toBe("ofox");
    expect(ofox.iconColor).toBe("#D97706");
    expect(ofox.websiteUrl).toBe("https://ofox.ai");
  });

  it("OfoxAI preset should have default models for all apps", () => {
    const ofox = universalProviderPresets[0];
    expect(ofox.defaultModels.claude).toBeDefined();
    expect(ofox.defaultModels.claude.model).toBeTruthy();
    expect(ofox.defaultModels.codex).toBeDefined();
    expect(ofox.defaultModels.codex.model).toBeTruthy();
    expect(ofox.defaultModels.gemini).toBeDefined();
    expect(ofox.defaultModels.gemini.model).toBeTruthy();
  });

  it("all presets should share the same AGGREGATOR_DEFAULT_MODELS reference", () => {
    // OfoxAI, NewAPI, and custom gateway should have identical model configs
    const ofoxModels = universalProviderPresets[0].defaultModels;
    const newApiPreset = universalProviderPresets.find(
      (p) => p.providerType === "newapi",
    );
    expect(newApiPreset).toBeDefined();
    // They share the same const, so should be reference-equal
    expect(ofoxModels).toBe(newApiPreset!.defaultModels);
  });
});
