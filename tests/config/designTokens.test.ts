import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

const css = fs.readFileSync(
  path.resolve(__dirname, "../../src/index.css"),
  "utf8",
);

function block(selector: string): Record<string, string> {
  const start = css.indexOf(`${selector} {`);
  expect(start, `${selector} block in index.css`).toBeGreaterThanOrEqual(0);
  const body = css.slice(start, css.indexOf("\n  }", start));
  const vars: Record<string, string> = {};
  for (const m of body.matchAll(/--([\w-]+):\s*([^;]+);/g)) {
    vars[m[1]] = m[2].replace(/\s+/g, " ").trim();
  }
  return vars;
}

const rgb = (hex: string) => {
  const n = parseInt(hex.slice(1), 16);
  return `${n >> 16} ${(n >> 8) & 255} ${n & 255}`;
};

// The palette from the design brief, verbatim.
const PALETTE = {
  ":root": {
    canvas: "#f4f4f6",
    "card-2": "#f9f9fb",
    pill: "#f1f1f4",
    "pill-hover": "#e9e9ee",
    line: "#e3e3e8",
    "line-2": "#ececf0",
    ink: "#1c1c21",
    "ink-2": "#4b4b53",
    "ink-muted": "#85858d",
    "ink-faint": "#b9b9c2",
    "ink-ctl": "#63636b",
    ok: "#189a58",
    "ok-soft": "#e4f6ec",
    danger: "#d64545",
    "danger-soft": "#fbe9e9",
    warn: "#b45309",
    "warn-soft": "#fbf0df",
    brand: "#ea580c",
    "brand-ink": "#c2410c",
    "brand-soft": "#fff1e8",
    "on-brand": "#ffffff",
  },
  ".dark": {
    canvas: "#1a1a1e",
    "card-2": "#1f1f23",
    pill: "#2d2d33",
    "pill-hover": "#36363d",
    line: "#333339",
    "line-2": "#2c2c32",
    ink: "#ededf1",
    "ink-2": "#b9b9c1",
    "ink-muted": "#8b8b94",
    "ink-faint": "#56565f",
    "ink-ctl": "#a1a1aa",
    ok: "#6fd99b",
    "ok-soft": "#1e3a2b",
    danger: "#f28b82",
    "danger-soft": "#45272a",
    warn: "#f2b544",
    "warn-soft": "#3f3220",
    brand: "#fb923c",
    "brand-ink": "#fdba74",
    "brand-soft": "#3a2618",
    "on-brand": "#1c1c21",
  },
} as const;

describe("design tokens", () => {
  for (const [selector, palette] of Object.entries(PALETTE)) {
    it(`${selector} carries the magpie palette with OFox orange`, () => {
      const vars = block(selector);
      for (const [name, hex] of Object.entries(palette)) {
        expect(vars[name], `--${name}`).toBe(rgb(hex));
      }
    });

    it(`${selector} primary and focus ring are the brand orange, not blue`, () => {
      const vars = block(selector);
      expect(vars.ring).toBe(vars.primary);
      const hue = Number(vars.primary.split(" ")[0]);
      expect(hue).toBeGreaterThanOrEqual(15);
      expect(hue).toBeLessThanOrEqual(35);
    });
  }

  it("focus-visible outlines use the brand colour", () => {
    expect(css).toMatch(
      /\*:focus-visible\s*{\s*outline:\s*2px solid rgb\(var\(--brand\)\);/,
    );
    expect(css).not.toMatch(/outline-blue/);
  });
});
