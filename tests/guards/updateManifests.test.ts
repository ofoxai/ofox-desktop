// @vitest-environment node
import { readFile } from "node:fs/promises";
import { describe, expect, it } from "vitest";
import {
  buildUpdateManifests,
  verifyPublishedManifests,
} from "../../scripts/update-manifests.mjs";

const base = "https://desktop.ofox.ai";
const mac = (version: string) =>
  `${base}/release/mac/arm/ofox_desktop_${version}.dmg`;
const windows = (version: string) =>
  `${base}/release/windows/x64/ofox_desktop_${version}.exe`;
const legacy = {
  version: "1.3.8",
  pubDate: "2026-10-26T04:15:00Z",
  notes: "",
  downloads: {
    "darwin-aarch64": mac("1.3.8"),
    "windows-x86_64": windows("1.3.8"),
  },
  downloadPage: "https://ofox.ai/download",
};
const input = {
  legacy,
  modern: null,
  version: "1.3.9",
  platforms: ["darwin-aarch64", "windows-x86_64"],
  pubDate: "2026-10-27T04:15:00Z",
  minimumMacOS: "13.5",
};
describe("dual update manifests", () => {
  it.each(["constructor", "__proto__", "unknown"])(
    "rejects an unregistered selected platform: %s",
    (platform) => {
      expect(() =>
        buildUpdateManifests({ ...input, platforms: [platform] }),
      ).toThrow();
    },
  );

  it("keeps the legacy Mac package while promoting Windows and the modern feed", () => {
    const result = buildUpdateManifests(input);
    expect(result.legacy.version).toBe("1.3.9");
    expect(result.legacy.downloads["darwin-aarch64"]).toBe(mac("1.3.8"));
    expect(result.legacy.downloads["windows-x86_64"]).toBe(windows("1.3.9"));
    expect(result.modern.downloads["darwin-aarch64"]).toBe(mac("1.3.9"));
    expect(result.modern.downloads["windows-x86_64"]).toBe(windows("1.3.9"));
    expect(result.modern).toMatchObject({
      schemaVersion: 2,
      minimumMacOS: "13.5",
    });
  });
  it("preserves unbuilt platforms in each feed across partial releases", () => {
    const prior = buildUpdateManifests(input);
    const result = buildUpdateManifests({
      ...input,
      legacy: prior.legacy,
      modern: prior.modern,
      version: "1.3.10",
      platforms: ["windows-x86_64"],
    });
    expect(result.modern.downloads["darwin-aarch64"]).toBe(mac("1.3.9"));
    expect(result.modern.downloads["windows-x86_64"]).toBe(windows("1.3.10"));
    expect(result.legacy.downloads["darwin-aarch64"]).toBe(mac("1.3.8"));
    const macOnly = buildUpdateManifests({
      ...input,
      platforms: ["darwin-aarch64"],
    });
    expect(macOnly.legacy.downloads["windows-x86_64"]).toBe(windows("1.3.8"));
  });
  it.each([
    undefined,
    null,
    {},
    { ...legacy, downloads: {} },
    {
      ...legacy,
      downloads: { ...legacy.downloads, "darwin-aarch64": mac("1.3.9") },
    },
  ])("refuses a missing or altered legacy Mac protection entry: %j", (bad) => {
    expect(() => buildUpdateManifests({ ...input, legacy: bad })).toThrow();
  });
  it.each(["1.3.7", "1.3.8", "invalid", "1.3.9-beta"])(
    "cannot overwrite the historical Mac release with %s",
    (version) => {
      expect(() => buildUpdateManifests({ ...input, version })).toThrow();
    },
  );
  it("refuses a downgrade of either feed", () => {
    const prior = buildUpdateManifests({ ...input, version: "1.3.10" });
    expect(() =>
      buildUpdateManifests({ ...input, modern: prior.modern }),
    ).toThrow();
    expect(() =>
      buildUpdateManifests({
        ...input,
        legacy: { ...legacy, version: "1.3.10" },
      }),
    ).toThrow();
  });
  it.each([
    "https://example.com/ofox_desktop_1.3.8.exe",
    windows("1.3.8") + "?redirect=x",
    windows("1.3.8") + "/extra",
  ])("does not republish an untrusted previous URL: %s", (url) => {
    expect(() =>
      buildUpdateManifests({
        ...input,
        legacy: {
          ...legacy,
          downloads: { ...legacy.downloads, "windows-x86_64": url },
        },
      }),
    ).toThrow();
  });
  it("refuses lost required platform entries instead of overwriting a partial snapshot", () => {
    expect(() =>
      buildUpdateManifests({
        ...input,
        legacy: { ...legacy, downloads: { "darwin-aarch64": mac("1.3.8") } },
      }),
    ).toThrow();
    const prior = buildUpdateManifests(input).modern;
    delete prior.downloads["darwin-aarch64"];
    expect(() =>
      buildUpdateManifests({
        ...input,
        modern: prior,
        platforms: ["windows-x86_64"],
      }),
    ).toThrow();
  });
  it("does not silently erase an unknown old platform", () => {
    expect(() =>
      buildUpdateManifests({
        ...input,
        legacy: {
          ...legacy,
          downloads: {
            ...legacy.downloads,
            "future-platform": "https://example.com/app",
          },
        },
      }),
    ).toThrow();
  });
  it("rejects malformed modern data instead of bootstrapping over it", () => {
    expect(() => buildUpdateManifests({ ...input, modern: {} })).toThrow();
    expect(() =>
      buildUpdateManifests({
        ...input,
        modern: { ...legacy, schemaVersion: 1 },
      }),
    ).toThrow();
  });
  it("checks both public feed contents, including the protected Mac URL", () => {
    const expected = buildUpdateManifests(input);
    expect(() =>
      verifyPublishedManifests({
        expected,
        published: structuredClone(expected),
      }),
    ).not.toThrow();
    const changed = structuredClone(expected);
    changed.legacy.downloads["darwin-aarch64"] = mac("1.3.9");
    expect(() =>
      verifyPublishedManifests({ expected, published: changed }),
    ).toThrow();
    changed.legacy = expected.legacy;
    changed.modern.minimumMacOS = "12.0";
    expect(() =>
      verifyPublishedManifests({ expected, published: changed }),
    ).toThrow();
  });
});

describe("release update-feed wiring", () => {
  it("routes CI through the shared conditional publisher", async () => {
    const workflow = await readFile(".github/workflows/release.yml", "utf8");
    expect(workflow).toContain("scripts/update-manifests.mjs publish");
    expect(workflow).toContain(
      '--source-sha "$GITHUB_SHA" --artifacts-root artifacts',
    );
    expect(workflow).not.toContain('aws s3 cp "$file"');
    expect(workflow).not.toContain("echo '{}' > old.json");
  });
  it("uses the same planner in emergency releases without an empty-manifest bypass", async () => {
    const script = await readFile("scripts/release.sh", "utf8");
    expect(script).toContain("scripts/update-manifests.mjs");
    expect(script).toContain("latest-v2.json");
    expect(script).not.toContain("FRESH_MANIFEST");
    expect(script).not.toContain("OLD_JSON='{}'");
  });
  it("moves new clients to the modern feed without changing the installed legacy reader", async () => {
    const source = await readFile("src-tauri/src/commands/misc.rs", "utf8");
    expect(source).toContain(
      'const OFOX_UPDATE_MANIFEST_URL: &str = "https://desktop.ofox.ai/latest-v2.json"',
    );
  });
});
