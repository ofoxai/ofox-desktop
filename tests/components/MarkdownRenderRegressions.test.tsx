import { render, waitFor } from "@testing-library/react";
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

/** The TeX source KaTeX keeps in each formula it drew. */
const sources = (container: HTMLElement) =>
  Array.from(
    container.querySelectorAll('annotation[encoding="application/x-tex"]'),
    (node) => node.textContent ?? "",
  );

async function drawn(container: HTMLElement, formulas: number) {
  await waitFor(() => expect(count(container, ".katex")).toBe(formulas));
}

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

describe("bare URLs in Chinese text", () => {
  it("keeps bold around a URL that Chinese punctuation follows", () => {
    const { container } = render(
      message("服务器已启动，地址是 **http://localhost:3000**，打开即可。"),
    );
    const link = container.querySelector("strong a");
    expect(link?.getAttribute("href")).toBe("http://localhost:3000/");
    expect(link?.textContent).toBe("http://localhost:3000");
    expect(prose(container)).not.toContain("**");
    expect(container.querySelector("p")?.textContent).toBe(
      "服务器已启动，地址是 http://localhost:3000，打开即可。",
    );
  });

  it("ends a link at the full stop that follows it", () => {
    const { container } = render(message("见 https://example.com/a。然后继续"));
    expect(container.querySelector("a")?.getAttribute("href")).toBe(
      "https://example.com/a",
    );
    expect(container.querySelector("p")?.textContent).toBe(
      "见 https://example.com/a。然后继续",
    );
  });
});

describe("display formulas", () => {
  it("keeps the text after a formula whose $$ share lines with it", async () => {
    const { container } = render(
      message(
        [
          "由包含关系",
          "",
          "$$\\frac12\\sin x<\\frac12 x",
          "\\;\\Longrightarrow\\;",
          "\\sin x<x$$",
          "",
          "同除以 $\\sin x>0$：",
          "",
          "## 数值上有多接近",
          "",
          "| $x$ | 值 |",
          "|---|---|",
          "| 1 | 0.84 |",
        ].join("\n"),
      ),
    );
    await drawn(container, 3);
    expect(count(container, ".katex-error")).toBe(0);
    expect(count(container, ".katex-display")).toBe(1);
    expect(sources(container)[0]).toContain("\\frac12\\sin x");
    expect(container.querySelector("h2")?.textContent).toBe("数值上有多接近");
    expect(count(container, "table")).toBe(1);
  });

  it("draws a $$ formula alone on its line as display math", async () => {
    const { container } = render(message("前文\n$$E=mc^2$$\n后文"));
    await drawn(container, 1);
    expect(count(container, ".katex-display")).toBe(1);
    expect(prose(container)).toContain("前文");
    expect(prose(container)).toContain("后文");
  });

  it("keeps the words after a closing $$ as text", async () => {
    const { container } = render(message("$$a+b\n=c$$ 所以成立。"));
    await drawn(container, 1);
    expect(count(container, ".katex-display")).toBe(1);
    expect(prose(container)).toContain("所以成立。");
  });

  it("draws such a formula inside a quote", async () => {
    const { container } = render(message("> $$a\n> =b$$\n\n后文"));
    await drawn(container, 1);
    expect(container.querySelector("blockquote .katex-display")).not.toBeNull();
    expect(prose(container)).toContain("后文");
  });

  it("shows a $$ that never closes as text instead of swallowing the rest", async () => {
    const { container } = render(message("$$x+1 与 $y$\n\n## 标题\n\n正文"));
    await drawn(container, 1);
    expect(container.querySelector("h2")?.textContent).toBe("标题");
    expect(prose(container)).toContain("$$x+1");
    expect(prose(container)).toContain("正文");
  });
});

describe("formulas in tables", () => {
  it("keeps the cells of a row whose formula holds a vertical bar", async () => {
    const { container } = render(
      message(
        "| 式子 | 含义 |\n|---|---|\n| $|x|$ | 绝对值 |\n| $\\|v\\|$ | 范数 |",
      ),
    );
    await drawn(container, 2);
    expect(count(container, "tbody td")).toBe(4);
    expect(count(container, ".katex-error")).toBe(0);
  });
});

describe("a formula KaTeX cannot draw", () => {
  it("is shown as quiet source text, not red", async () => {
    const { container } = render(message("看 $$\\frac{1}{$$ 这里"));
    await waitFor(() =>
      expect(container.querySelector(".katex-error")).not.toBeNull(),
    );
    const error = container.querySelector(".katex-error") as HTMLElement;
    expect(error.style.color).not.toMatch(/cc0000|204,\s*0,\s*0/i);
    expect(error.getAttribute("title")).toMatch(/KaTeX/);
  });
});
