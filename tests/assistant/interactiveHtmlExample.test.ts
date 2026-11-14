import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { fireEvent } from "@testing-library/react";

function loadExample() {
  const skill = readFileSync(
    "assistant-runtime/skills/ofox-office-html/SKILL.md",
    "utf8",
  );
  const html = /```html\n([\s\S]*?)\n```/.exec(skill)?.[1];
  if (!html) throw new Error("Interactive HTML example is missing");
  const parsed = new DOMParser().parseFromString(html, "text/html");
  document.documentElement.innerHTML = parsed.documentElement.innerHTML;
  const code = document.querySelector("script")?.textContent;
  if (!code) throw new Error("Interactive HTML behavior is missing");
  // Only the independently authored, bundled example is executed here.
  new Function("document", code)(document);
  const input = document.querySelector<HTMLInputElement>("#query")!;
  const buttons = [
    ...document.querySelectorAll<HTMLButtonElement>("[data-filter]"),
  ];
  const visible = () =>
    [...document.querySelectorAll<HTMLLIElement>("#results>li")].filter(
      (item) => !item.hidden,
    );
  return { input, buttons, visible };
}

describe("bundled interactive HTML behavior", () => {
  it("combines search/category, announces empty results and resets without losing keyboard focus", () => {
    const { input, buttons, visible } = loadExample();
    expect(visible()).toHaveLength(3);
    fireEvent.click(
      buttons.find((button) => button.dataset.filter === "build")!,
    );
    expect(visible()).toHaveLength(1);
    expect(document.querySelector('[data-filter="build"]')).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    input.value = "missing";
    fireEvent.input(input);
    expect(visible()).toHaveLength(0);
    expect(document.querySelector("#empty")).not.toHaveAttribute("hidden");
    expect(document.querySelector('[role="status"]')).toHaveTextContent(
      "0 of 3",
    );
    fireEvent.click(document.querySelector("#reset")!);
    expect(visible()).toHaveLength(3);
    expect(input.value).toBe("");
    expect(document.activeElement).toBe(input);
    expect(document.querySelector('[data-filter="all"]')).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("handles Enter without navigation and exposes named native controls", () => {
    const { input, visible } = loadExample();
    input.value = "example";
    const event = new Event("submit", { bubbles: true, cancelable: true });
    expect(document.querySelector("form")!.dispatchEvent(event)).toBe(false);
    expect(visible()).toHaveLength(1);
    expect(input.closest("label")).toHaveTextContent("Search options");
    expect(document.querySelector('meta[name="viewport"]')).not.toBeNull();
    expect(
      document.querySelectorAll('script[src],link[rel="stylesheet"]').length,
    ).toBe(0);
  });
});
