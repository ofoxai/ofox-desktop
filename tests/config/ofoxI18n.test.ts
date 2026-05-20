import { describe, expect, it } from "vitest";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";

describe("OfoxAI i18n keys", () => {
  const locales = { zh, en, ja } as Record<string, any>;

  for (const [lang, data] of Object.entries(locales)) {
    it(`${lang}.json should have firstRunNotice.bodyOfox`, () => {
      expect(data.firstRunNotice.bodyOfox).toBeDefined();
      expect(typeof data.firstRunNotice.bodyOfox).toBe("string");
      expect(data.firstRunNotice.bodyOfox.length).toBeGreaterThan(0);
    });

    it(`${lang}.json should have provider.ofoxQuickStart`, () => {
      expect(data.provider.ofoxQuickStart).toBeDefined();
      expect(typeof data.provider.ofoxQuickStart).toBe("string");
      expect(data.provider.ofoxQuickStart.length).toBeGreaterThan(0);
    });
  }

  it("bodyOfox should mention ofox.ai in all locales", () => {
    for (const [lang, data] of Object.entries(locales)) {
      expect(
        data.firstRunNotice.bodyOfox.toLowerCase(),
        `${lang} bodyOfox should mention ofox.ai`,
      ).toContain("ofox.ai");
    }
  });

  it("ofoxQuickStart should mention OfoxAI in all locales", () => {
    for (const [lang, data] of Object.entries(locales)) {
      expect(
        data.provider.ofoxQuickStart,
        `${lang} ofoxQuickStart should mention OfoxAI`,
      ).toContain("OfoxAI");
    }
  });
});
