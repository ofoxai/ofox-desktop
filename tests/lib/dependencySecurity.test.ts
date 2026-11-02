import { describe, expect, it } from "vitest";
import katex, { type KatexOptions } from "katex";
import { parse as parseToml } from "smol-toml";
import { z } from "zod";
import { createRequire } from "node:module";

const packageRequire = createRequire(import.meta.url);
const markdownRequire = createRequire(packageRequire.resolve("rehype-katex"));
const markdownKatex = markdownRequire("katex") as typeof katex;

describe("dependency security boundaries", () => {
  describe.each([
    { entry: "direct", renderer: katex },
    { entry: "Markdown", renderer: markdownKatex },
  ])("$entry KaTeX dependency", ({ renderer }) => {
    it("ignores trusted rendering inherited from an options prototype", () => {
      const options = Object.create({ trust: true }) as KatexOptions;
      const html = renderer.renderToString(
        String.raw`\href{javascript:alert(1)}{link}`,
        options,
      );

      expect(html).not.toMatch(/<a\b/);
    });

    it("preserves an explicit trusted HTTPS rendering option", () => {
      const html = renderer.renderToString(
        String.raw`\href{https://example.invalid/docs}{link}`,
        { trust: true },
      );

      expect(html).toContain('<a href="https://example.invalid/docs"');
    });
  });

  const collections = [
    {
      kind: "array",
      create: (element: z.ZodString) => z.array(element).max(1),
      oversized: ["first", "second"],
      allowed: ["first"],
    },
    {
      kind: "set",
      create: (element: z.ZodString) => z.set(element).max(1),
      oversized: new Set(["first", "second"]),
      allowed: new Set(["first"]),
    },
  ];

  it.each(collections)(
    "rejects an oversized $kind before visiting its entries",
    ({ create, oversized, allowed }) => {
      let visits = 0;
      const element = z.string().refine(() => {
        visits += 1;
        return true;
      });
      const schema = create(element);
      const result = schema.safeParse(oversized);

      expect(result.success).toBe(false);
      expect(visits).toBe(0);
      if (result.success) throw new Error("oversized collection was accepted");
      expect(result.error.issues).toMatchObject([{ code: "too_big" }]);

      expect(schema.safeParse(allowed).success).toBe(true);
      expect(visits).toBeGreaterThan(0);
    },
  );

  it("parses a bounded TOML input with many consecutive comment lines", () => {
    const input = "# comment\n".repeat(12_000) + "enabled = true\n";

    expect(parseToml(input)).toMatchObject({ enabled: true });
  });
});
