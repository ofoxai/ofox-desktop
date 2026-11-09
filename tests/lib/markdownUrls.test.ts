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
});
