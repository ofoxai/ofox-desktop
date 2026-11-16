import { render, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import katex from "katex";
import zh from "@/i18n/locales/zh.json";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { loadMarkdownMath } from "@/lib/markdownMath";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  await loadMarkdownMath();
});

const reply = [
  "第一段，有公式 $a^2$。",
  "- 列表一\n- 列表二",
  "$$\nE = mc^2\n$$",
  "| 列 | 值 |\n|---|---|\n| $x$ | 1 |",
  "最后一段在写 $b$ 和更多文字。",
].join("\n\n");

const view = (content: string, streaming: boolean) => (
  <SessionMarkdown content={content} math streaming={streaming} />
);

/** The markdown's top-level elements, one per block. */
const blocks = (container: HTMLElement) =>
  Array.from((container.firstElementChild as HTMLElement).children);

async function settled(container: HTMLElement) {
  await waitFor(() =>
    expect(container.querySelector(".katex, .katex-error")).not.toBeNull(),
  );
}

describe("a streaming reply", () => {
  it("draws every finished block as a fresh render of the same text does", async () => {
    for (let cut = 20; cut <= reply.length; cut += 7) {
      const text = reply.slice(0, cut);
      const live = render(view(text, true));
      const fresh = render(view(text, false));
      if (text.includes("$a^2$")) {
        await settled(live.container);
        await settled(fresh.container);
      }
      const now = blocks(live.container);
      const then = blocks(fresh.container);
      expect(now.length).toBe(then.length);
      for (let index = 0; index < now.length - 1; index += 1)
        expect(now[index].outerHTML, `cut ${cut}, block ${index}`).toBe(
          then[index].outerHTML,
        );
      live.unmount();
      fresh.unmount();
    }
  });

  it("shows the formulas of the block being written as source", async () => {
    const { container } = render(view(reply, true));
    await settled(container);
    const shown = blocks(container);
    const last = shown[shown.length - 1];
    expect(last.querySelector(".katex")).toBeNull();
    expect(last.querySelector("code.language-math")?.textContent).toBe("b");
    expect(shown[0].querySelector(".katex")).not.toBeNull();
    expect(container.querySelector(".katex-error")).toBeNull();
  });

  it("shows a display formula still being written as source", async () => {
    for (const tail of [
      "$$\n\\frac12\\sin x<\n\\sin",
      "$$\\frac12\\sin x<\n\\sin",
    ]) {
      const { container, unmount } = render(
        view(`前文 $a$。\n\n${tail}`, true),
      );
      await settled(container);
      const shown = blocks(container);
      const last = shown[shown.length - 1];
      expect(last.tagName).toBe("PRE");
      expect(last.textContent).toContain("\\frac12\\sin x<");
      expect(last.textContent).toContain("\\sin");
      expect(container.querySelector(".katex-error")).toBeNull();
      unmount();
    }
  });

  it("draws the last block's formulas once the reply has finished", async () => {
    const { container, rerender } = render(view(reply, true));
    await settled(container);
    rerender(view(reply, false));
    const shown = blocks(container);
    await waitFor(() =>
      expect(shown[shown.length - 1].querySelector(".katex")).not.toBeNull(),
    );
  });

  it("does not draw finished blocks again while the last one grows", async () => {
    const { container, rerender } = render(view(reply, true));
    await settled(container);
    const draw = vi.spyOn(katex, "renderToString");
    rerender(view(`${reply}再写一点`, true));
    rerender(view(`${reply}再写一点，还有一点`, true));
    expect(draw).not.toHaveBeenCalled();
    draw.mockRestore();
  });
});

describe("blocks that hang on the line before them", () => {
  it("reads a list right under a paragraph as the whole text does", () => {
    const { container } = render(
      view("新增三个步骤：\n- 07. 生成语音\n- 08. 加号菜单\n\n后文 $x$", false),
    );
    expect(container.querySelector("ul > li")?.querySelector("ol")).toBeNull();
    expect(container.querySelector("ul > li")?.textContent).toBe(
      "07. 生成语音",
    );
  });
});

describe("review follow-ups", () => {
  /** Every block but the last, streaming, is the same as the text drawn whole. */
  async function sameAsWhole(text: string) {
    const live = render(view(text, true));
    const fresh = render(view(text, false));
    await settled(live.container);
    await settled(fresh.container);
    const now = blocks(live.container);
    const then = blocks(fresh.container);
    expect(now.length).toBe(then.length);
    for (let index = 0; index < now.length - 1; index += 1)
      expect(now[index].outerHTML).toBe(then[index].outerHTML);
    live.unmount();
    fresh.unmount();
  }

  it("keeps the indent of a block's first line", async () => {
    await sameAsWhole("段落 $x$\n\n  ```\n  code\n    deeper\n  ```\n\n后文");
    await sameAsWhole("段落 $x$\n\n  - a\n   - b\n\n后文");
  });

  it("draws formulas while a reply with a reference definition streams", async () => {
    const { container } = render(
      view(
        "一 $a$。\n\n二 $b$，见[文档][1]。\n\n三 $c$。\n\n[1]: https://a.example",
        true,
      ),
    );
    await waitFor(() =>
      expect(container.querySelectorAll(".katex").length).toBe(3),
    );
    expect(container.querySelector("code.language-math")).toBeNull();
  });

  it("cuts a reply with Windows line breaks into blocks", async () => {
    const { container } = render(view("一 $a$。\r\n\r\n二 $b$", true));
    await settled(container);
    expect(blocks(container)[0].querySelector(".katex")).not.toBeNull();
  });

  it("shows a \\[ formula still being written as source", async () => {
    const { container } = render(view("前文 $a$。\n\n\\[\n\\frac12\n", true));
    await settled(container);
    const shown = blocks(container);
    expect(shown[shown.length - 1].tagName).toBe("PRE");
    expect(shown[shown.length - 1].textContent).toContain("\\frac12");
  });

  it("keeps a math code block's box and copy button where math is not drawn", () => {
    const { container } = render(
      <SessionMarkdown content={"```math\nx^2\n```"} />,
    );
    expect(container.querySelector("button[aria-label]")).not.toBeNull();
  });
});

describe("a reply that is still one block", () => {
  it("shows the formulas of its only block as source", async () => {
    for (const text of ["$$\n\\frac12\\sin x<\n\\sin", "\\[\n\\frac12"]) {
      const { container, unmount } = render(view(text, true));
      await waitFor(() => expect(blocks(container)[0]?.tagName).toBe("PRE"));
      unmount();
    }
    const { container } = render(view("单段 $a$ 和 \\(b\\) 还在写", true));
    await waitFor(() =>
      expect(container.querySelectorAll("code.language-math").length).toBe(2),
    );
    expect(container.querySelector(".katex")).toBeNull();
  });

  it("leaves indented code and a closed \\[ as they are", () => {
    const code = render(view("前文\n\n    \\[x\\]\n    y", true));
    expect(code.container.querySelector("pre code")?.textContent).toBe(
      "\\[x\\]\ny\n",
    );
  });
});

describe("a reply whose parts refer to each other", () => {
  it("resolves a reference link defined in another block", () => {
    const { container } = render(
      view("见[文档][1]，$x$。\n\n[1]: https://a.example/doc", true),
    );
    expect(container.querySelector("a")?.getAttribute("href")).toBe(
      "https://a.example/doc",
    );
  });

  it("keeps a footnote with its definition", () => {
    const { container } = render(view("正文[^1]。\n\n[^1]: 注释", false));
    expect(container.querySelector("section[data-footnotes]")).not.toBeNull();
  });
});
