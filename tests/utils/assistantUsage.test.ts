import { createInstance } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import {
  fmtDuration,
  replyDuration,
  usageSummary,
} from "@/utils/assistantUsage";

const i18n = createInstance();
beforeAll(async () => {
  await i18n.init({
    lng: "en",
    resources: { en: { translation: en } },
    interpolation: { escapeValue: false },
  });
});

describe("usageSummary", () => {
  it("is nothing without usage or tokens", () => {
    expect(usageSummary(undefined, true, i18n.t)).toBeNull();
    expect(usageSummary({}, true, i18n.t)).toBeNull();
  });

  it("gives the tokens alone while the cost is not settled", () => {
    expect(
      usageSummary(
        { inputTokens: 1200, outputTokens: 300, estimatedCostUsd: "0.5" },
        false,
        i18n.t,
      ),
    ).toBe("Input 1.2K · Output 300 tokens");
  });

  it("adds the estimated cost once settled", () => {
    expect(
      usageSummary(
        { inputTokens: 3, outputTokens: 312, estimatedCostUsd: "0.0004" },
        true,
        i18n.t,
      ),
    ).toBe("Input 3 · Output 312 tokens · Estimated cost <$0.001");
  });

  it("counts the search charges a cost includes", () => {
    expect(
      usageSummary(
        {
          inputTokens: 10,
          outputTokens: 10,
          estimatedCostUsd: "0.03",
          searchCount: 2,
        },
        true,
        i18n.t,
      ),
    ).toContain("incl. 2 search charges");
  });

  it("says the cost is unavailable, never $0, when a request's cost is unknown", () => {
    expect(
      usageSummary(
        {
          inputTokens: 10,
          outputTokens: 10,
          estimatedCostUsd: "0",
          usageCoverage: "incomplete",
        },
        true,
        i18n.t,
      ),
    ).toBe("Input 10 · Output 10 tokens · Cost unavailable");
    expect(
      usageSummary({ inputTokens: 10, outputTokens: 5 }, true, i18n.t),
    ).toBe("Input 10 · Output 5 tokens · Cost unavailable");
  });
});

describe("replyDuration", () => {
  it("is how long the reply took, from its start to its end", () => {
    expect(replyDuration({ createdAt: 1_000, completedAt: 5_200 })).toBe(4_200);
  });

  it("is unknown for a reply without an end, or with one before its start", () => {
    expect(replyDuration({ createdAt: 1_000 })).toBeNull();
    expect(replyDuration({ createdAt: 1_000, completedAt: null })).toBeNull();
    expect(replyDuration({ createdAt: 2_000, completedAt: 1_000 })).toBeNull();
  });
});

describe("fmtDuration", () => {
  it("keeps a decimal for the first ten seconds only", () => {
    expect(fmtDuration(4_240, "en")).toBe("4.2 sec");
    expect(fmtDuration(42_400, "en")).toBe("42 sec");
  });

  it("never says zero", () => {
    expect(fmtDuration(0, "en")).toBe("<1 sec");
    expect(fmtDuration(999, "en")).toBe("<1 sec");
  });

  it("gives minutes and seconds from a minute on", () => {
    expect(fmtDuration(72_000, "en")).toBe("1 min 12 sec");
    expect(fmtDuration(120_000, "en")).toBe("2 min");
  });

  it("follows the language", () => {
    expect(fmtDuration(4_200, "zh")).toContain("秒");
    expect(fmtDuration(72_000, "ja")).toContain("分");
  });
});
