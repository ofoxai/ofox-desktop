// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";

const directory = new URL("../../src-tauri/", import.meta.url);
const configurations = readdirSync(directory)
  .filter((name) => /^tauri(?:\.[^.]+)?\.conf\.json$/.test(name))
  .map((name) => ({
    name,
    value: JSON.parse(readFileSync(new URL(name, directory), "utf8")),
  }));
const base = configurations.find(({ name }) => name === "tauri.conf.json")!;

function frameSources(policy: string): string[] {
  const directives = policy
    .split(";")
    .map((directive) => directive.trim().split(/\s+/));
  const frames = directives.filter(([name]) => name === "frame-src");
  expect(frames).toHaveLength(1);
  return frames[0].slice(1).sort();
}

describe("HTML preview frame navigation policy", () => {
  it.each(configurations)(
    "$name permits only inert srcdoc and the isolated lease origin as frames",
    ({ value }) => {
      const policy = value.app?.security?.csp ?? base.value.app.security.csp;
      expect(typeof policy).toBe("string");
      expect(frameSources(policy)).toEqual([
        "about:",
        "http://127.0.0.1:43763",
      ]);
    },
  );

  it("does not treat the application origin or arbitrary local services as child-frame destinations", () => {
    const frames = frameSources(base.value.app.security.csp);
    for (const forbidden of [
      "'self'",
      "*",
      "http:",
      "https:",
      "http://127.0.0.1:*",
      "http://localhost:43763",
      "asset:",
      "tauri:",
    ]) {
      expect(frames).not.toContain(forbidden);
    }
  });
});
