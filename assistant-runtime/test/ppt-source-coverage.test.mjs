import test from "node:test";
import assert from "node:assert/strict";
import { existsSync } from "node:fs";
import { readFile } from "node:fs/promises";
import {
  activationProofAccepted,
  assertOriginalBytes,
  LEGACY_ACTIVATION,
  verifyCoverage,
} from "../../scripts/presentation-resource-coverage.mjs";

const manifest = new URL(
  "../skills/ofox-office-pptx/resources/source-coverage.json",
  import.meta.url,
);

test("original deck coverage distinguishes actual upstream pages from eight-page Ofox samples", async () => {
  assert.ok(
    existsSync(manifest),
    "A machine-readable original-deck coverage record is required",
  );
  const coverage = JSON.parse(await readFile(manifest, "utf8"));
  assert.deepEqual(
    coverage.artifactKinds.map((kind) => kind.id),
    [
      "originalSourceDeck",
      "nativeAdaptation",
      "selfAuthoredExample",
      "generatedUserFile",
    ],
  );
  assert.equal(coverage.styles.length, 25);
  for (const [id, count] of Object.entries({
    "blue-professional": 10,
    cartesian: 10,
    "course-module": 7,
    "product-launch": 8,
    "architectural-plan": 6,
  })) {
    const style = coverage.styles.find((entry) => entry.id === id);
    assert.equal(style.sourceDeck.pageCount, count, id);
    assert.equal(style.sourceDeck.pages.length, count, id);
    // Every actual page is individually classified; native sample pages are
    // Ofox's own content and never stand in for the upstream deck.
    assert.equal(style.nativeAdaptation.status, "page-mapped", id);
    assert.equal(
      style.nativeAdaptation.adapted +
        style.nativeAdaptation.approximate +
        style.nativeAdaptation.unsupported,
      count,
      id,
    );
    assert.equal(style.selfAuthoredPreview, null, id);
    assert.equal(style.nativeAdaptationSample.sourceEquivalent, false, id);
  }
  const navy = coverage.styles.find((entry) => entry.id === "navy");
  assert.equal(navy.selfAuthoredPreview.pageCount, 8);
  assert.equal(navy.selfAuthoredPreview.sourceEquivalent, false);
  assert.equal(navy.sourceDeck.pageCount, 0);
  assert.equal(navy.sourceDeck.layoutCount, 3);
  assert.deepEqual(
    navy.sourceDeck.layouts.map((layout) => layout.label),
    ["Cover", "Section", "Content"],
  );
  assert.equal(
    coverage.styles.filter((entry) => entry.availability === "candidate")
      .length,
    0,
    "every catalog style has been through adaptation",
  );
  assert.equal(coverage.artifactKinds.at(-1).staticResources, false);
});

test("resource verifier reads original bytes and rejects stale page counts, false sample equivalence and source hashes", async () => {
  const coverage = JSON.parse(await readFile(manifest, "utf8"));
  assert.equal(
    (await verifyCoverage(undefined, coverage)).originalEnabledPages,
    236,
  );
  for (const mutate of [
    (value) => {
      value.styles.find(
        (entry) => entry.id === "course-module",
      ).sourceDeck.pageCount = 8;
    },
    (value) => {
      value.styles.find(
        (entry) => entry.id === "cartesian",
      ).nativeAdaptationSample.sourceEquivalent = true;
    },
    (value) => {
      value.sources[0].revision = "0".repeat(40);
    },
    (value) => {
      value.artifactKinds.at(-1).staticResources = true;
    },
    (value) => {
      value.styles.find((entry) => entry.id === "navy").sourceDeck.layoutCount =
        8;
    },
  ]) {
    const changed = structuredClone(coverage);
    mutate(changed);
    await assert.rejects(verifyCoverage(undefined, changed), /manifest drift/);
  }
  const resource = coverage.styles
    .find((entry) => entry.id === "blue-professional")
    .sourceDeck.files.find((file) => file.path.endsWith(".html"));
  const bytes = await readFile(
    new URL(
      resource.path,
      new URL("../skills/ofox-office-pptx/resources/", import.meta.url),
    ),
  );
  assert.doesNotThrow(() => assertOriginalBytes(bytes, resource));
  assert.throws(
    () =>
      assertOriginalBytes(
        Buffer.concat([bytes, Buffer.from("changed")]),
        resource,
      ),
    /hash mismatch/,
  );
});

test("actual CSS/script dependencies and separate third-party license boundaries remain visible", async () => {
  const coverage = JSON.parse(await readFile(manifest, "utf8"));
  const cartesian = coverage.styles.find((entry) => entry.id === "cartesian");
  assert.ok(
    cartesian.sourceDeck.dependencies.some(
      (dependency) =>
        dependency.url === "https://cdn.jsdelivr.net/npm/chart.js" &&
        dependency.status === "not-fetched",
    ),
  );
  const course = coverage.styles.find((entry) => entry.id === "course-module");
  assert.equal(
    course.sourceDeck.pages[0].selector,
    ".deck > .slide",
    "Runtime overview clones are not original pages",
  );
  assert.equal(
    course.sourceDeck.dependencies.filter(
      (dependency) => dependency.kind === "remote-font",
    ).length,
    8,
  );
  assert.ok(
    course.sourceDeck.dependencyFiles.some((file) =>
      file.path.endsWith("assets/runtime.js"),
    ),
  );
  for (const style of coverage.styles.filter((entry) => entry.sourceId)) {
    assert.equal(style.licenses.code.status, "reviewed");
    for (const kind of ["fonts", "images", "brands"])
      assert.equal(style.licenses[kind].status, "pending");
  }
});

test("both original Cartesian canvas pages are genuinely rendered with a pinned, licensed preview-only dependency", async () => {
  const coverage = JSON.parse(await readFile(manifest, "utf8"));
  const cartesian = coverage.styles.find((entry) => entry.id === "cartesian");
  for (const number of [4, 7]) {
    const page = cartesian.sourceDeck.preview.pages.find(
      (entry) => entry.number === number,
    );
    assert.ok(
      page,
      `Actual upstream Cartesian chart page ${number} must not be omitted or replaced by an Ofox sample`,
    );
    assert.equal(page.chartEvidence.libraryVersion, "4.6.1");
    assert.equal(page.chartEvidence.datasets.length, 2);
    assert.ok(
      page.chartEvidence.nonTransparentPixels > 1000,
      "The original canvas contains rendered ink",
    );
  }
  const dependency = coverage.previewDependencies.find(
    (entry) => entry.id === "chartjs-preview",
  );
  assert.equal(dependency.version, "4.6.1");
  assert.equal(dependency.license, "MIT");
  assert.equal(dependency.scope, "offline-source-preview-only");
  assert.equal(dependency.originalAuthorVersionKnown, false);
});

test("every later original capture proves its page was visibly shown, not just counted", async () => {
  const evidence = JSON.parse(
    await readFile(
      new URL(
        "../skills/ofox-office-pptx/resources/source-preview-evidence.json",
        import.meta.url,
      ),
      "utf8",
    ),
  );
  const later = evidence.styles.filter(
    (style) => !LEGACY_ACTIVATION.has(style.id),
  );
  assert.equal(later.length, 19);
  for (const style of later) {
    assert.equal(style.renderer.activationMode, "native-navigation", style.id);
    for (const page of style.pages)
      assert.ok(
        activationProofAccepted(page.activationProof),
        `${style.id}/${page.number} ${JSON.stringify(page.activationProof)}`,
      );
  }
  assert.equal(
    activationProofAccepted({
      viewportCoverage: 1,
      textElements: 12,
      visibleTextElements: 0,
    }),
    false,
  );
});
