import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

function config(file: string) {
  return JSON.parse(readFileSync(path.resolve("src-tauri", file), "utf8"));
}

describe("native title and custom header", () => {
  it("hides the native macOS title in the overlay even when a QA or future window sets a title", () => {
    const main = config("tauri.conf.json").app.windows.find(
      (window: { label: string }) => window.label === "main",
    );
    expect(main.titleBarStyle).toBe("Overlay");
    expect(main.hiddenTitle).toBe(true);
  });

  it("keeps the normal Windows title bar and product title visible", () => {
    const base = config("tauri.conf.json");
    const main = config("tauri.windows.conf.json").app.windows.find(
      (window: { label: string }) => window.label === "main",
    );
    expect(main.titleBarStyle).toBe("Visible");
    expect(main.title).toBe(base.productName);
    expect(main.hiddenTitle).toBe(false);
  });
});
