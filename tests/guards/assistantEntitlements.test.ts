// @vitest-environment node
import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import {
  ENTITLEMENTS,
  entitlementsFor,
} from "../../scripts/assistant-entitlements.mjs";

const runtime = path.resolve(__dirname, "../../assistant-runtime");
const engines = path.join(runtime, "resources", "engines");
const downloads = JSON.parse(
  readFileSync(path.join(runtime, "engine-downloads.json"), "utf8"),
);
const entitlementKeys = (plist: string) =>
  [
    ...readFileSync(path.join(runtime, plist), "utf8").matchAll(
      /<key>(.*?)<\/key>/g,
    ),
  ].map((match) => match[1]);

describe("assistant executable entitlements", () => {
  it("signs Node and officecli with JIT only and typst with none", () => {
    expect(
      entitlementsFor(path.join(engines, "officecli", "officecli"), engines),
    ).toBe("officecli-entitlements.plist");
    expect(
      entitlementsFor(path.join(engines, "typst", "typst"), engines),
    ).toBeNull();
    expect(
      entitlementsFor(path.join(runtime, "resources", "node", "node"), engines),
    ).toBe("node-entitlements.plist");
    for (const plist of [
      "node-entitlements.plist",
      "officecli-entitlements.plist",
    ])
      expect(entitlementKeys(plist), plist).toEqual([
        "com.apple.security.cs.allow-jit",
      ]);
  });

  it("fails the build for an engine executable with no entry", () => {
    expect(() =>
      entitlementsFor(path.join(engines, "newengine", "newengine"), engines),
    ).toThrow(/newengine.*no entry in ENTITLEMENTS/);
  });

  it("keeps signing other Mach-O files outside the engines without entitlements", () => {
    expect(
      entitlementsFor(
        path.join(runtime, "resources", "runtime", "skia.darwin-arm64.node"),
        engines,
      ),
    ).toBeNull();
  });

  it("lists every engine executable the manifest pins for macOS", () => {
    for (const name of ["officecli", "typst"])
      for (const target of ["darwin-arm64", "darwin-x64"]) {
        const executable = path.basename(downloads[name][target].executable);
        expect(
          Object.hasOwn(ENTITLEMENTS, executable),
          `${name} ${target}`,
        ).toBe(true);
      }
  });
});
