import { describe, expect, it } from "vitest";
import {
  hasMathMarkers,
  loadMarkdownMath,
  markdownMathReady,
  rewriteMathDelimiters,
} from "@/lib/markdownMath";

describe("hasMathMarkers", () => {
  it.each(["$$x^2$$", "a \\(x\\) b", "\\[\nx\n\\]"])("sees %j", (content) => {
    expect(hasMathMarkers(content)).toBe(true);
  });

  it.each(["", "plain text", "it costs $5 and $10", "a (b) and [c]"])(
    "does not see %j",
    (content) => {
      expect(hasMathMarkers(content)).toBe(false);
    },
  );
});

describe("rewriteMathDelimiters", () => {
  it("writes \\(…\\) as text math", () => {
    expect(rewriteMathDelimiters("area \\(\\pi r^2\\) here")).toBe(
      "area $$\\pi r^2$$ here",
    );
  });

  it("writes a \\[ … \\] block as a $$ block", () => {
    expect(rewriteMathDelimiters("a\n\\[\nE = mc^2\n\\]\nb")).toBe(
      "a\n$$\nE = mc^2\n$$\nb",
    );
  });

  it("puts the formula of a one-line \\[…\\] between the fences", () => {
    expect(rewriteMathDelimiters("\\[ E = mc^2 \\]")).toBe(
      "$$\n E = mc^2 \n$$",
    );
  });

  it("keeps the text on the fence lines of a block with the formula", () => {
    expect(rewriteMathDelimiters("\\[ a +\nb \\]")).toBe("$$\n a +\nb \n$$");
  });

  it("keeps a block quote's marker on the fences", () => {
    expect(rewriteMathDelimiters("> \\[\n> x\n> \\]")).toBe("> $$\n> x\n> $$");
  });

  it("writes \\[…\\] in the middle of a line as text math", () => {
    expect(rewriteMathDelimiters("see \\[x + 1\\] above")).toBe(
      "see $$x + 1$$ above",
    );
  });

  it("pads a formula that starts or ends with a dollar", () => {
    expect(rewriteMathDelimiters("\\($x$\\)")).toBe("$$ $x$ $$");
  });

  it("keeps adjacent formulas apart", () => {
    expect(rewriteMathDelimiters("\\(x\\)\\(y\\)")).toBe("$$x$$ $$y$$");
  });

  it("keeps a formula apart from a dollar before or after it", () => {
    expect(rewriteMathDelimiters("$\\(x\\)$5")).toBe("$ $$x$$ $5");
  });

  it("leaves a message above the size cap as it was", () => {
    const small = "\\(x\\) ";
    const text = small.repeat(Math.ceil((64 * 1024 + 1) / small.length));
    expect(text.length).toBeGreaterThan(64 * 1024);
    expect(rewriteMathDelimiters(text)).toBe(text);
    expect(rewriteMathDelimiters(text.slice(0, 1000))).toContain("$$x$$");
  });

  it("leaves a formula that holds $$ as it was", () => {
    expect(rewriteMathDelimiters("\\(a $$ b\\)")).toBe("\\(a $$ b\\)");
  });

  it("leaves an escaped backslash and an unclosed delimiter alone", () => {
    const text = "a \\\\(b\\\\) and \\(open and \\[open";
    expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("does not close a formula across a blank line", () => {
    const text = "\\(a\n\nb\\)";
    expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("leaves \\\\[ inside a formula as a line break", () => {
    expect(rewriteMathDelimiters("\\(a \\\\[2pt] b\\)")).toBe(
      "$$a \\\\[2pt] b$$",
    );
  });

  it.each([
    ["fenced code", "```\n\\(x\\)\n\\[\ny\n\\]\n```"],
    ["tilde fence", "~~~js\nconst a = /\\(x\\)/;\n~~~"],
    ["a fence with Windows line breaks", "~~~\r\n\\(x\\)\r\n~~~"],
    ["a fence inside a list", "- item\n\n  ```\n  \\(x\\)\n  ```"],
    ["a fence in a quote", "> ```\n> \\(x\\)\n> ```"],
    ["inline code", "use `\\(x\\)` and ``a \\[b\\] c`` here"],
    ["indented code", "text\n\n    \\(x\\)\n    \\[y\\]\n\nafter"],
  ])("leaves %s as it was", (_name, text) => {
    expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("rewrites after a fence has closed", () => {
    expect(rewriteMathDelimiters("```\n\\(x\\)\n```\n\\(y\\)")).toBe(
      "```\n\\(x\\)\n```\n$$y$$",
    );
  });

  it("rewrites an indented line that continues a list item", () => {
    expect(rewriteMathDelimiters("- one\n\n    \\(y\\)")).toBe(
      "- one\n\n    $$y$$",
    );
  });

  it("does not take a line of inline code for a fence", () => {
    expect(rewriteMathDelimiters("```a``` \\(x\\)")).toBe("```a``` $$x$$");
  });

  it("leaves a lone backtick and plain text as they were", () => {
    const text = "a ` b and (c) [d] $5";
    expect(rewriteMathDelimiters(text)).toBe(text);
  });
});

describe("loadMarkdownMath", () => {
  it("loads the plugins once and reports ready", async () => {
    const first = loadMarkdownMath();
    expect(loadMarkdownMath()).toBe(first);
    await first;
    expect(markdownMathReady()).toBe(true);
    expect(loadMarkdownMath()).toBe(first);
  });
});
