import { describe, expect, it } from "vitest";
import { rewriteBareUrls } from "@/lib/markdownUrls";

describe("rewriteBareUrls", () => {
  it("ends a URL at the Chinese punctuation after it, and the bold around it", () => {
    expect(rewriteBareUrls("地址是 **http://localhost:3000**，打开即可")).toBe(
      "地址是 **<http://localhost:3000>**，打开即可",
    );
    expect(rewriteBareUrls("见 https://example.com/a。然后")).toBe(
      "见 <https://example.com/a>。然后",
    );
    expect(rewriteBareUrls("（官网：https://ofox.ai/zh）")).toBe(
      "（官网：<https://ofox.ai/zh>）",
    );
  });

  it("writes a www address as a link to its http page", () => {
    expect(rewriteBareUrls("打开 www.example.com，然后")).toBe(
      "打开 [www.example.com](http://www.example.com)，然后",
    );
  });

  it("leaves a URL that markdown already ends right as it was", () => {
    for (const text of [
      "see https://example.com/a. Then",
      "**http://localhost:3000** 打开",
      "https://zh.wikipedia.org/wiki/佐々木",
      "地址https://a.com",
    ])
      expect(rewriteBareUrls(text)).toBe(text);
  });

  it("leaves links, autolinks and code as they were", () => {
    for (const text of [
      "[官网](https://ofox.ai/zh，)",
      "<https://example.com/a>。",
      "`https://example.com/a，b`",
      "```\nhttps://example.com/a，b\n```",
    ])
      expect(rewriteBareUrls(text)).toBe(text);
  });

  it("rewrites every URL in the text", () => {
    expect(rewriteBareUrls("先 https://a.com，再 https://b.com。")).toBe(
      "先 <https://a.com>，再 <https://b.com>。",
    );
  });

  it("stops a URL at a table cell's bar", () => {
    const text = "|官网|https://ofox.ai|中文（推荐）|";
    expect(rewriteBareUrls(text)).toBe(text);
  });

  it("leaves link text, reference definitions and HTML attributes as they were", () => {
    for (const text of [
      "[官网 https://a.com，看](https://b.com)",
      "[1]: https://a.com（官方）",
      '<a href="https://a.com，">官网</a>',
    ])
      expect(rewriteBareUrls(text)).toBe(text);
  });

  it("ends a URL at a Chinese closing quote", () => {
    expect(rewriteBareUrls("访问“https://ofox.ai/zh”，即可")).toBe(
      "访问“<https://ofox.ai/zh>”，即可",
    );
  });

  it("still ends a URL after a bracket that is not link text", () => {
    expect(rewriteBareUrls("区间 [0, 1) 内，见 https://a.com，然后")).toBe(
      "区间 [0, 1) 内，见 <https://a.com>，然后",
    );
  });

  it("does not take a less-than sign in a sentence for a tag", () => {
    expect(rewriteBareUrls("当 x<y 时访问 **https://a.com**，再看 z>0")).toBe(
      "当 x<y 时访问 **<https://a.com>**，再看 z>0",
    );
  });

  it("leaves comments and link text with brackets inside as they were", () => {
    for (const text of [
      "<!-- 见 https://a.com，c -->",
      "[见 https://a.com，详见[1]](https://b.com)",
    ])
      expect(rewriteBareUrls(text)).toBe(text);
  });
});
