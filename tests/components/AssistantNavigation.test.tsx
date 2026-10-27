import { expect, it } from "vitest";
import { SHELL_PAGES } from "@/components/shell/ShellContext";

it("places the built-in assistant after tools, sessions, library and usage", () => {
  expect(SHELL_PAGES).toEqual([
    "tools",
    "sessions",
    "library",
    "usage",
    "assistant",
  ]);
});
