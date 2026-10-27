import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import { loadMarkdownSyntax } from "@/lib/markdownSyntax";
import * as clipboard from "@/lib/clipboard";

beforeEach(async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
});

function message(content: string) {
  return <SessionMarkdown content={content} syntaxHighlight />;
}

/** Waits until the lazily loaded grammars have highlighted the first block. */
async function highlighted(container: HTMLElement) {
  await waitFor(() =>
    expect(container.querySelector("pre code.hljs")).not.toBeNull(),
  );
}

describe("assistant code blocks", () => {
  it("shows code as readable text at once and highlights it once the grammars load", async () => {
    const { container } = render(message("```ts\nconst ready = true;\n```"));
    expect(container.querySelector("pre code")?.textContent).toBe(
      "const ready = true;\n",
    );
    await highlighted(container);
    expect(container.querySelector("pre code")?.textContent).toBe(
      "const ready = true;\n",
    );
  });

  it.each([
    ["ts", 'const answer: string = "hello";'],
    ["python", 'def greet():\n    return "hello"'],
    ["json", '{"answer": true}'],
    ["bash", 'echo "hello"'],
  ])(
    "highlights explicitly labelled %s without changing the copied source",
    async (language, source) => {
      const copy = vi.spyOn(clipboard, "copyText").mockResolvedValue();
      const { container } = render(
        message(`\`\`\`${language}\n${source}\n\`\`\``),
      );
      await highlighted(container);
      const code = container.querySelector("pre code");
      expect(code?.querySelector('span[class^="hljs-"]')).not.toBeNull();
      expect(code?.textContent).toBe(`${source}\n`);
      await userEvent.click(screen.getByRole("button", { name: "复制代码" }));
      expect(copy).toHaveBeenCalledWith(source);
    },
  );

  it("keeps legacy external-session rendering unchanged unless explicitly enabled", async () => {
    await loadMarkdownSyntax();
    const { container } = render(
      <SessionMarkdown
        content={"```ts\nconst answer = 42;\n```"}
        highlight="answer"
      />,
    );
    expect(container.querySelector("mark")).toHaveTextContent("answer");
    expect(container.querySelector(".hljs")).toBeNull();
    expect(container.querySelector(".markdown-syntax")).toBeNull();
  });

  it("leaves unknown languages and unlabelled blocks as readable text", async () => {
    await loadMarkdownSyntax();
    const { container } = render(
      message(
        "```unknown-ofox-language\nconst plain = true;\n```\n\n```\nconst unlabelled = true;\n```",
      ),
    );
    expect(container.querySelectorAll("pre")).toHaveLength(2);
    expect(container.querySelector(".hljs")).toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe(
      "const plain = true;\n",
    );
  });

  it("keeps long blocks fully readable and copyable while bounding highlighting work", async () => {
    const large = 'const row = "long";\n'.repeat(2000);
    const { container } = render(
      message(
        `\`\`\`ts\n${large}\`\`\`\n\n\`\`\`ts\nconst small = true;\n\`\`\``,
      ),
    );
    await highlighted(container);
    const blocks = container.querySelectorAll("pre code");
    expect(blocks[0].textContent).toBe(large);
    expect(blocks[0].querySelector("span")).toBeNull();
    expect(blocks[1].querySelector("span")).not.toBeNull();
  });

  it("does not execute code or load embedded HTML and remote images", async () => {
    const source =
      '<script>window.stolen = true</script><img src="https://tracker.invalid/pixel">';
    const { container } = render(
      message(
        `\`\`\`html\n${source}\n\`\`\`\n\n${source}\n\n![image](https://tracker.invalid/pixel)`,
      ),
    );
    await highlighted(container);
    expect(container.querySelector("script, img")).toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe(
      `${source}\n`,
    );
    expect(container.querySelector("pre code span")).not.toBeNull();
  });

  it("bounds total highlighted text per message while retaining every code block", async () => {
    const source = `const answer = 42; // ${"a".repeat(24000)}`;
    const blocks = [source, source, source, "const tail = true;"];
    const { container } = render(
      message(blocks.map((block) => `\`\`\`ts\n${block}\n\`\`\``).join("\n\n")),
    );
    await highlighted(container);
    const code = container.querySelectorAll("pre code");
    expect(Array.from(code, (node) => node.textContent)).toEqual(
      blocks.map((block) => `${block}\n`),
    );
    expect(Array.from(code, (node) => !!node.querySelector("span"))).toEqual([
      true,
      true,
      false,
      true,
    ]);
  });

  it("preserves incomplete streamed code and inline code without interpreting either", async () => {
    await loadMarkdownSyntax();
    const { container, rerender } = render(
      message(
        'Inline `const plain = 1;`.\n\n```ts\nconst result = "unfinished',
      ),
    );
    expect(container.querySelector("p code span")).toBeNull();
    expect(container.querySelector("pre code")?.textContent).toBe(
      'const result = "unfinished\n',
    );
    rerender(
      message(
        'Inline `const plain = 1;`.\n\n```ts\nconst result = "finished";\n```',
      ),
    );
    expect(container.querySelector("pre code")?.textContent).toBe(
      'const result = "finished";\n',
    );
  });
});
