import { render } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";

// Replies as models write them, each a shape that once rendered wrong. The
// text is made up; the shapes come from real replies.

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

const message = (content: string) => <SessionMarkdown content={content} math />;

/** The text a reader sees, without code and formulas. */
function prose(container: HTMLElement): string {
  const copy = container.cloneNode(true) as HTMLElement;
  copy.querySelectorAll("code, pre, .katex").forEach((node) => node.remove());
  return copy.textContent ?? "";
}

const count = (container: HTMLElement, selector: string) =>
  container.querySelectorAll(selector).length;

describe("Chinese emphasis", () => {
  it("closes bold that ends on full-width punctuation before Chinese text", () => {
    const { container } = render(
      message(
        [
          "**说明：**正文第一句。",
          "**多头注意力（Multi-Head Attention）**后面接正文。",
          "现在卡在**「推送镜像」**这一步。",
          '还剩**"只有一个任务"**，这个最难。',
          "**计费：**refusal 按输入计费。",
        ].join("\n\n"),
      ),
    );
    expect(count(container, "strong")).toBe(5);
    expect(prose(container)).not.toContain("**");
  });

  it("closes a strikethrough the same way", () => {
    const { container } = render(message("~~旧方案：~~新方案。"));
    expect(count(container, "del")).toBe(1);
    expect(prose(container)).not.toContain("~~");
  });
});
