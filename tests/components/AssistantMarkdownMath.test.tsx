import { render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import * as markdownMath from "@/lib/markdownMath";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

afterEach(() => {
  vi.restoreAllMocks();
});

function message(content: string, highlight = "") {
  return <SessionMarkdown content={content} highlight={highlight} math />;
}

/** The TeX source KaTeX keeps in each formula it drew. */
function sources(container: HTMLElement) {
  return Array.from(
    container.querySelectorAll('annotation[encoding="application/x-tex"]'),
    (node) => node.textContent,
  );
}

async function rendered(container: HTMLElement) {
  await waitFor(() => expect(container.querySelector(".katex")).not.toBeNull());
}

describe("assistant math", () => {
  it("draws a $$ formula inside a sentence once KaTeX loads", async () => {
    const { container } = render(message("area $$\\pi r^2$$ ok"));
    await rendered(container);
    expect(sources(container)).toEqual(["\\pi r^2"]);
    expect(container.querySelector(".katex-display")).toBeNull();
  });

  it("draws a $$ block as display math", async () => {
    const { container } = render(message("a\n\n$$\nE = mc^2\n$$\n\nb"));
    await rendered(container);
    expect(container.querySelector(".katex-display")).not.toBeNull();
    expect(sources(container)).toEqual(["E = mc^2"]);
  });

  it("draws \\(…\\) as an inline formula", async () => {
    const { container } = render(message("area \\(\\pi r^2\\) ok"));
    await rendered(container);
    expect(sources(container)).toEqual(["\\pi r^2"]);
    expect(container.querySelector("p")?.textContent).toContain("area ");
    expect(container.querySelector("p")?.textContent).toContain(" ok");
  });

  it("draws \\[…\\] on its own lines as display math", async () => {
    const { container } = render(
      message("sum:\n\\[\n\\sum_{i=1}^{n} i\n\\]\ndone"),
    );
    await rendered(container);
    expect(container.querySelector(".katex-display")).not.toBeNull();
    expect(sources(container)).toEqual(["\\sum_{i=1}^{n} i"]);
  });

  it("draws a one-line \\[…\\] as display math", async () => {
    const { container } = render(message("\\[ a^2 + b^2 = c^2 \\]"));
    await rendered(container);
    expect(container.querySelector(".katex-display")).not.toBeNull();
  });

  it("draws $…$ as an inline formula, as models write it", async () => {
    const { container } = render(message("序列长度 $n$，模型维度 $d$。"));
    await rendered(container);
    expect(sources(container)).toEqual(["n", "d"]);
    expect(container.querySelector(".katex-display")).toBeNull();
  });

  it("leaves $5 and $10 as plain text, even beside a real formula", async () => {
    const { container } = render(message("costs $5 and $10, $$x$$"));
    await rendered(container);
    expect(sources(container)).toEqual(["x"]);
    expect(container.querySelector("p")?.textContent).toContain(
      "costs $5 and $10, ",
    );
  });

  it("does not load KaTeX for text without a math marker", () => {
    const load = vi.spyOn(markdownMath, "loadMarkdownMath");
    const { container } = render(message("costs $5 and $10 (a) [b]"));
    expect(load).not.toHaveBeenCalled();
    expect(container.querySelector("p")?.textContent).toBe(
      "costs $5 and $10 (a) [b]",
    );
  });

  it("does not load KaTeX for a caller that did not ask for math", () => {
    const load = vi.spyOn(markdownMath, "loadMarkdownMath");
    const { container } = render(
      <SessionMarkdown content={"area $$x$$ and \\(y\\)"} />,
    );
    expect(load).not.toHaveBeenCalled();
    expect(container.querySelector(".katex")).toBeNull();
  });

  it("keeps external transcripts as they were even once KaTeX is loaded", async () => {
    await markdownMath.loadMarkdownMath();
    const { container } = render(
      <SessionMarkdown content={"area $$x$$ and \\(y\\)"} />,
    );
    expect(container.querySelector(".katex")).toBeNull();
    expect(container.querySelector("p")?.textContent).toBe(
      "area $$x$$ and (y)",
    );
  });

  it("leaves code blocks and inline code untouched", async () => {
    await markdownMath.loadMarkdownMath();
    const code = "const a = /\\(x\\)/;\n$$not math$$\n\\[\nb\n\\]";
    const { container } = render(
      message(`\`\`\`js\n${code}\n\`\`\`\n\nuse \`\\(y\\)\` and \`$$z$$\``),
    );
    expect(container.querySelector(".katex")).toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe(`${code}\n`);
    expect(
      Array.from(container.querySelectorAll("p code"), (n) => n.textContent),
    ).toEqual(["\\(y\\)", "$$z$$"]);
  });

  it("draws a formula beside code without touching the code", async () => {
    const { container } = render(
      message("\\(a\\)\n\n```\n\\(b\\)\n```\n\n$$c$$"),
    );
    await waitFor(() => expect(sources(container)).toEqual(["a", "c"]));
    expect(container.querySelector("pre code")?.textContent).toBe("\\(b\\)\n");
  });

  it("marks a search word in the text but never inside a formula", async () => {
    const { container } = render(message("x and \\(x^2 + x\\) and x", "x"));
    await rendered(container);
    expect(container.querySelectorAll("mark")).toHaveLength(2);
    expect(container.querySelector(".katex mark")).toBeNull();
    expect(sources(container)).toEqual(["x^2 + x"]);
  });

  it("shows the TeX as text, without throwing, when a formula is malformed", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const { container } = render(message("bad $$\\frac{1$$ and \\(\\bogus\\)"));
    await waitFor(() =>
      expect(container.querySelector(".katex-error")).not.toBeNull(),
    );
    expect(container.querySelector("p")?.textContent).toContain("\\frac{1");
    expect(warn).not.toHaveBeenCalled();
    expect(error).not.toHaveBeenCalled();
  });

  it("draws Chinese text and accents in a formula without console noise", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const { container } = render(message("$$面积 = \\pi r^2 + é$$"));
    await rendered(container);
    expect(sources(container)).toEqual(["面积 = \\pi r^2 + é"]);
    expect(warn).not.toHaveBeenCalled();
  });

  it("does not run a formula's links or HTML", async () => {
    const { container } = render(
      message(
        "$$\\href{javascript:alert(1)}{x}$$ $$\\includegraphics{https://t.invalid/p.png}$$ $$\\url{https://t.invalid}$$",
      ),
    );
    await rendered(container);
    expect(container.querySelector("a, img, script")).toBeNull();
  });

  it("caps the size a formula can take", async () => {
    const { container } = render(message("$$\\rule{9999em}{9999em}$$"));
    await rendered(container);
    expect(container.querySelector("[style*='9999em']")).toBeNull();
    expect(container.querySelector("[style*='20em']")).not.toBeNull();
  });

  it("does not expand a macro bomb", async () => {
    const { container } = render(message("$$\\def\\a{\\a\\a}\\a$$"));
    await waitFor(() =>
      expect(container.querySelector(".katex-error")).not.toBeNull(),
    );
  });

  it("keeps drawing after the content changes while streaming", async () => {
    const { container, rerender } = render(message("a \\(x"));
    expect(container.querySelector("p")?.textContent).toBe("a (x");
    rerender(message("a \\(x\\) b \\(y"));
    await waitFor(() => expect(sources(container)).toEqual(["x"]));
    rerender(message("a \\(x\\) b \\(y\\)"));
    await waitFor(() => expect(sources(container)).toEqual(["x", "y"]));
  });
});
