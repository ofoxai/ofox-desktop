import { describe, expect, it } from "vitest";
import { splitUrlTail } from "@/lib/urlTail";

describe("where a URL ends in running text", () => {
  it.each([
    ["。", "。"],
    ["，然后", "，然后"],
    ["）", "）"],
    ["」", "」"],
    ["、", "、"],
    ["！", "！"],
    ["：", "："],
    ["；", "；"],
    ["？", "？"],
    ["…", "…"],
    ["【注】", "【注】"],
    ["　后面", "　后面"],
    ["｡", "｡"],
    ["”，即可", "”，即可"],
    ["’s", "’s"],
  ])("cuts at the first full-width mark: %s", (rest, tail) => {
    expect(splitUrlTail(`https://a.example/x${rest}`)).toEqual({
      url: "https://a.example/x",
      tail,
    });
  });

  it("cuts at the first mark and keeps everything after it as the tail", () => {
    expect(
      splitUrlTail("https://a.example/x，见https://b.example/y。"),
    ).toEqual({
      url: "https://a.example/x",
      tail: "，见https://b.example/y。",
    });
  });

  it("drops ASCII punctuation just before a full-width mark with it", () => {
    expect(splitUrlTail("https://a.example/x.。")).toEqual({
      url: "https://a.example/x",
      tail: ".。",
    });
  });

  it("drops trailing ASCII sentence punctuation from a URL with no CJK mark", () => {
    expect(splitUrlTail("https://a.example/x?q=1.")).toEqual({
      url: "https://a.example/x?q=1",
      tail: ".",
    });
    expect(splitUrlTail("https://a.example/x;,")).toEqual({
      url: "https://a.example/x",
      tail: ";,",
    });
  });

  it.each([
    ["https://a.example/x)。", "https://a.example/x", ")。"],
    ["https://a.example/x)，然后", "https://a.example/x", ")，然后"],
    ['https://a.example/x"。', "https://a.example/x", '"。'],
    ["https://a.example/x'。", "https://a.example/x", "'。"],
    ["https://a.example/x_。然后", "https://a.example/x", "_。然后"],
    ["https://a.example/x**。", "https://a.example/x", "**。"],
    ["https://a.example/x~~。", "https://a.example/x", "~~。"],
    ["https://a.example/x]。", "https://a.example/x", "]。"],
    ["https://a.example/x.)。", "https://a.example/x", ".)。"],
  ])(
    "after a full-width cut, also drops the closers that went with it: %s",
    (found, url, tail) => {
      expect(splitUrlTail(found)).toEqual({ url, tail });
    },
  );

  it("keeps the parentheses of an address that balances them", () => {
    expect(splitUrlTail("https://en.wikipedia.org/wiki/Foo_(bar)。")).toEqual({
      url: "https://en.wikipedia.org/wiki/Foo_(bar)",
      tail: "。",
    });
    expect(splitUrlTail("https://en.wikipedia.org/wiki/Foo_(bar))。")).toEqual({
      url: "https://en.wikipedia.org/wiki/Foo_(bar)",
      tail: ")。",
    });
  });

  it("leaves closers alone when no full-width mark ends the URL", () => {
    expect(splitUrlTail("https://a.example/x_")).toEqual({
      url: "https://a.example/x_",
      tail: "",
    });
  });

  it.each([
    "https://a.example/x",
    "https://zh.wikipedia.org/wiki/东京",
    "https://ja.wikipedia.org/wiki/佐々木小次郎",
    "https://ja.wikipedia.org/wiki/東京〆",
    "https://a.example/ＡＢ１",
    "https://a.example/ひらがな・カタカナー",
  ])(
    "leaves %s whole: ideographs, kana, letters and numerals are not marks",
    (url) => {
      expect(splitUrlTail(url)).toEqual({ url, tail: "" });
    },
  );
});
