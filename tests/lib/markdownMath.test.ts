import { describe, expect, it } from "vitest";
import {
  hasMathMarkers,
  loadMarkdownMath,
  markdownMathPlugins,
  markdownMathReady,
  rewriteMathDelimiters,
} from "@/lib/markdownMath";

describe("hasMathMarkers", () => {
  it.each(["$$x^2$$", "a \\(x\\) b", "\\[\nx\n\\]", "序列长度 $n$，维度 $d$"])(
    "sees %j",
    (content) => {
      expect(hasMathMarkers(content)).toBe(true);
    },
  );

  it.each([
    "",
    "plain text",
    "it costs $5 and $10",
    "a (b) and [c]",
    "约 $0.014–$0.028 每次",
    "$5,$10",
    "price: $ 5 $",
  ])("does not see %j", (content) => {
    expect(hasMathMarkers(content)).toBe(false);
  });
});

describe("rewriteMathDelimiters", () => {
  it("writes $…$ as text math, as Pandoc reads it", () => {
    expect(
      rewriteMathDelimiters("长度 $n$，输入 $X\\in\\mathbb{R}^{n\\times d}$。"),
    ).toBe("长度 $$n$$，输入 $$X\\in\\mathbb{R}^{n\\times d}$$。");
    expect(rewriteMathDelimiters("若 $X' = PX$（$P$ 为置换）")).toBe(
      "若 $$X' = PX$$（$$P$$ 为置换）",
    );
  });

  it("leaves money and spaced dollars as text", () => {
    for (const text of [
      "it costs $5 and $10",
      "约 $0.014–$0.028 每次",
      "$5,$10 or $20/month",
      "a $ b $ c",
      "escaped \\$x\\$ stays",
    ])
      expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("does not close a $ formula across a blank line or inside code", () => {
    expect(rewriteMathDelimiters("$a\n\nb$")).toBe("$a\n\nb$");
    expect(rewriteMathDelimiters("`$x$` and $y$")).toBe("`$x$` and $$y$$");
  });

  it("keeps $$ formulas as they are", () => {
    expect(rewriteMathDelimiters("$$a $b$ c$$ and $d$")).toBe(
      "$$a $b$ c$$ and $$d$$",
    );
  });

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

describe("rewriteMathDelimiters: money beside Chinese text", () => {
  it("leaves two prices with Chinese words between them as text", () => {
    const text = "token × $0.412/M。里的「补偿$待核」要等这个数";
    expect(rewriteMathDelimiters(text)).toBe(text);
    expect(hasMathMarkers(text)).toBe(false);
  });

  it("still reads a formula whose Chinese words sit in \\text", () => {
    expect(rewriteMathDelimiters("$\\text{面积}=\\pi r^2$")).toBe(
      "$$\\text{面积}=\\pi r^2$$",
    );
  });
});

describe("rewriteMathDelimiters: $$ blocks", () => {
  it("puts the fences of a formula that shares their lines on lines of their own", () => {
    expect(rewriteMathDelimiters("由\n\n$$a<b\n\\Rightarrow\nc$$\n\n后")).toBe(
      "由\n\n$$\na<b\n\\Rightarrow\nc\n$$\n\n后",
    );
  });

  it("writes a $$ formula alone on its line as a block", () => {
    expect(rewriteMathDelimiters("$$E=mc^2$$")).toBe("$$\nE=mc^2\n$$");
    expect(rewriteMathDelimiters("前文\n$$E=mc^2$$\n后文")).toBe(
      "前文\n$$\nE=mc^2\n$$\n后文",
    );
  });

  it("puts the words after a closing $$ on a line after the block", () => {
    expect(rewriteMathDelimiters("$$a+b\n=c$$ 所以成立。")).toBe(
      "$$\na+b\n=c\n$$\n所以成立。",
    );
  });

  it("keeps a quote's marker and a list item's indent on every line", () => {
    expect(rewriteMathDelimiters("> $$a\n> =b$$\n\n后文")).toBe(
      "> $$\n> a\n> =b\n> $$\n\n后文",
    );
    expect(rewriteMathDelimiters("1. 推导：\n\n   $$a\n   =b$$")).toBe(
      "1. 推导：\n\n   $$\n   a\n   =b\n   $$",
    );
  });

  it("leaves a block whose fences stand alone as it was, blank lines and all", () => {
    for (const text of ["$$\nx\n$$", "a\n\n$$\na\n\nb\n$$\n\nb"])
      expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("keeps a $$ formula inside a sentence inline", () => {
    for (const text of ["面积 $$\\pi r^2$$ 即可", "$$a$$ 与 $$b$$ 相等"])
      expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("shows a $$ that never closes as text", () => {
    expect(rewriteMathDelimiters("$$x+1\n\n## 标题")).toBe(
      "\\$\\$x+1\n\n## 标题",
    );
    expect(rewriteMathDelimiters("a $$x")).toBe("a \\$\\$x");
  });

  it("does not close a $$ that has words after it across a blank line", () => {
    expect(rewriteMathDelimiters("$$x\n\n正文 $$y$$")).toBe(
      "\\$\\$x\n\n正文 $$y$$",
    );
  });

  it("writes the vertical bars of a formula in a table row as \\vert", () => {
    expect(
      rewriteMathDelimiters(
        "| 式 | 义 |\n|---|---|\n| $|x|$ | 绝对值 |\n| $\\|v\\|$ | 范数 |",
      ),
    ).toBe(
      "| 式 | 义 |\n|---|---|\n| $$\\vert x\\vert $$ | 绝对值 |\n| $$\\Vert v\\Vert $$ | 范数 |",
    );
    expect(rewriteMathDelimiters("$|x|$")).toBe("$$|x|$$");
  });
});

describe("rewriteMathDelimiters: review follow-ups", () => {
  it("still reads formulas with Chinese in subscripts and bold", () => {
    expect(
      rewriteMathDelimiters(
        "合力 $F_{合}=ma$，速度 $v_{平均}$，区间 $x\\in[0，1]$，$\\mathbf{力}$",
      ),
    ).toBe(
      "合力 $$F_{合}=ma$$，速度 $$v_{平均}$$，区间 $$x\\in[0，1]$$，$$\\mathbf{力}$$",
    );
    expect(hasMathMarkers("速度 $v_{平均}$")).toBe(true);
  });

  it("does not pair dollars across the rows of a table", () => {
    for (const [text, out] of [
      [
        "| $$ | PID |\n|---|---|\n| $$x$$ | 公式 |",
        "| \\$\\$ | PID |\n|---|---|\n| $$x$$ | 公式 |",
      ],
      [
        "| a | b |\n|---|---|\n| $x | 1 |\n| y$ | 2 |",
        "| a | b |\n|---|---|\n| $x | 1 |\n| y$ | 2 |",
      ],
    ])
      expect(rewriteMathDelimiters(text)).toBe(out);
  });

  it("reads a one-column table's delimiter row", () => {
    expect(rewriteMathDelimiters("| 式 |\n|---\n| $|x|$ |")).toBe(
      "| 式 |\n|---\n| $$\\vert x\\vert $$ |",
    );
  });

  it("keeps an escaped backslash before a bar in a table formula", () => {
    expect(rewriteMathDelimiters("| a |\n|---|\n| $a \\\\| b$ |")).toBe(
      "| a |\n|---|\n| $$a \\\\\\vert  b$$ |",
    );
  });

  it("writes a formula in a list item as a block under the item", () => {
    expect(rewriteMathDelimiters("- $$a\n  =b$$")).toBe(
      "- $$\n  a\n  =b\n  $$",
    );
    const fenced = "- $$\n  a\n\n  b\n  $$";
    expect(rewriteMathDelimiters(fenced)).toBe(fenced);
  });

  it("writes a formula opened mid-sentence and closed on its own line as a block", () => {
    expect(rewriteMathDelimiters("公式为 $$\na+b\n$$\n\n后文")).toBe(
      "公式为\n$$\na+b\n$$\n\n后文",
    );
  });

  it("leaves a block whose formula is indented four spaces as it was", () => {
    const text = "$$\n\n    E = mc^2\n\n$$";
    expect(rewriteMathDelimiters(text)).toBe(text);
  });

  it("keeps the words moved after a closing $$ from starting a list or heading", () => {
    expect(rewriteMathDelimiters("$$a\n=b$$ - 1")).toBe("$$\na\n=b\n$$\n\\- 1");
    expect(rewriteMathDelimiters("$$a\n=b$$ # 注")).toBe(
      "$$\na\n=b\n$$\n\\# 注",
    );
  });

  it("asks KaTeX for muted errors", async () => {
    await loadMarkdownMath();
    const rehype = markdownMathPlugins()?.rehype ?? [];
    const options = (
      rehype[0] as unknown as [unknown, { errorColor?: string }]
    )[1];
    expect(options.errorColor).toBe("rgb(var(--ink-muted))");
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
