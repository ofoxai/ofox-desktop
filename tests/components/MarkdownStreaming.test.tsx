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
