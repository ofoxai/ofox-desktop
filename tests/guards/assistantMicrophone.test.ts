// @vitest-environment node
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";
it("ships a specific microphone purpose and localized native permission strings", () => {
  const read = (name: string) =>
    readFileSync(new URL(`../../src-tauri/${name}`, import.meta.url), "utf8");
  expect(read("Info.plist")).toContain("NSMicrophoneUsageDescription");
  const base = JSON.parse(read("tauri.conf.json")).bundle.resources;
  const windows = JSON.parse(read("tauri.windows.conf.json")).bundle.resources;
  for (const locale of ["en", "zh-Hans", "ja"]) {
    const source = `infoplist/${locale}.lproj/InfoPlist.strings`;
    expect(base[source]).toBe(`${locale}.lproj/InfoPlist.strings`);
    expect(windows[source]).toBeNull();
    expect(read(source)).toContain('"NSMicrophoneUsageDescription" = "');
  }
});
