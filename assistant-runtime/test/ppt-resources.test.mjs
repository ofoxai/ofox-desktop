import test from "node:test";
import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";

const root = new URL("../skills/ofox-office-pptx/resources/", import.meta.url);
test("bundled public assets retain pinned provenance, license text and original source hashes", async () => {
  const catalog = JSON.parse(
    await readFile(new URL("catalog.json", root), "utf8"),
  );
  const sources = new Map(catalog.sources.map((source) => [source.id, source]));
  const resources = new Map();
  for (const owner of [...catalog.sources, ...catalog.styles]) {
    for (const file of [
      ...(owner.resources ?? []),
      ...(owner.licenseFiles ?? []),
    ]) {
      assert.match(file.sha256, /^[a-f0-9]{64}$/);
      assert.ok(
        file.path.startsWith("vendor/") && !file.path.split("/").includes(".."),
      );
      const bytes = await readFile(new URL(file.path, root));
      assert.equal(
        createHash("sha256").update(bytes).digest("hex"),
        file.sha256,
        file.path,
      );
      if (resources.has(file.path))
        assert.equal(resources.get(file.path), file.sha256);
      resources.set(file.path, file.sha256);
    }
  }
  assert.equal(
    new Set(catalog.styles.map((style) => style.id)).size,
    catalog.styles.length,
  );
  for (const source of sources.values()) {
    assert.match(source.repository, /^https:\/\/github.com\//);
    assert.match(source.revision, /^[a-f0-9]{40}$/);
    assert.ok(resources.has(source.licenseFile));
  }
  for (const style of catalog.styles) {
    assert.ok(sources.has(style.sourceId));
    assert.ok(style.resources.length > 0);
    assert.ok(style.licenseFiles.length > 0);
    if (style.enabled) {
      assert.ok(style.licenseReviewed && style.adapted && style.theme);
      assert.equal(style.status, "enabled");
      assert.equal(
        style.verified,
        false,
        "Quality acceptance requires actual experiment evidence",
      );
      assert.equal(style.validation.visualQuality, "pending");
    }
  }
});
