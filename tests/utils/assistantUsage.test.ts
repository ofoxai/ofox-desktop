import { createInstance } from "i18next";
import { beforeAll, describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import {
  fmtDuration,
  fmtUsd,
  replyDuration,
  usageBreakdown,
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

describe("usageBreakdown", () => {
  const request = (cost: Record<string, number>) => ({
    kind: "chat",
    status: "completed",
    usage: { cost },
  });
  it("lists each priced line with its tokens, from the requests' frozen prices", () => {
    const breakdown = usageBreakdown({
      inputTokens: 3,
      outputTokens: 42,
      cacheReadTokens: 17_700,
      cacheWriteTokens: 0,
      estimatedCostUsd: "0.000162",
      requests: [
        request({ input: 0.000004, output: 0.00003, cacheRead: 0.0001 }),
        request({ input: 0.000002, output: 0.00002, cacheRead: 0.000006 }),
      ],
    });
    expect(breakdown.lines.map((l) => l.key)).toEqual([
      "input",
      "cacheRead",
      "output",
    ]);
    expect(breakdown.lines[1]).toEqual({
      key: "cacheRead",
      tokens: 17_700,
      usd: 0.000106,
    });
    expect(breakdown.lines[2].usd).toBeCloseTo(0.00005, 9);
    expect(breakdown.total).toBe(0.000162);
  });
  it("adds web searches and leaves every amount unknown when one request's cost is", () => {
    const usage = {
      inputTokens: 10,
      outputTokens: 5,
      searchCount: 2,
      searchCostUsd: "0.016",
      estimatedCostUsd: "0.0161",
      requests: [request({ input: 0.00005, output: 0.00005 })],
    };
    expect(usageBreakdown(usage).search).toEqual({ count: 2, usd: 0.016 });
    const unknown = usageBreakdown({ ...usage, usageCoverage: "incomplete" });
    expect(unknown.total).toBeNull();
    expect(unknown.lines.every((l) => l.usd === null)).toBe(true);
    expect(unknown.search?.usd).toBeNull();
  });
});

describe("fmtUsd", () => {
  it("keeps enough digits for a fraction of a cent, as ofox-agent does", () => {
    expect(fmtUsd(0)).toBe("$0.00");
    expect(fmtUsd(0.00005)).toBe("$0.000050");
    expect(fmtUsd(0.0046)).toBe("$0.0046");
    expect(fmtUsd(0.016)).toBe("$0.02");
    expect(fmtUsd(1.5)).toBe("$1.50");
    expect(fmtUsd(null)).toBe("—");
  });
});
