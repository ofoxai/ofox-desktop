// @vitest-environment node
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("ships the audio decoder license and source notice on both bundle platforms", () => {
  const read = (file: string) =>
    JSON.parse(
      readFileSync(new URL(`../../src-tauri/${file}`, import.meta.url), "utf8"),
    );
  const base = read("tauri.conf.json").bundle.resources;
  const windows = read("tauri.windows.conf.json").bundle.resources;
  for (const name of ["SYMPHONIA-MPL-2.0.txt", "SYMPHONIA-NOTICE.md"]) {
    const source = `../docs/licenses/${name}`;
    const destination = `licenses/${name}`;
    expect(base[source]).toBe(destination);
    expect({ ...base, ...windows }[source]).toBe(destination);
    expect(
      readFileSync(
        new URL(`../../docs/licenses/${name}`, import.meta.url),
        "utf8",
      ).length,
    ).toBeGreaterThan(500);
  }
});
