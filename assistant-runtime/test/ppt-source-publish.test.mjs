import test from "node:test";
import assert from "node:assert/strict";
import { readFile, writeFile, mkdir, mkdtemp, rm, cp } from "node:fs/promises";
import { createHash } from "node:crypto";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { publishSourcePreviewBatch } from "../../scripts/source-preview-publish.mjs";
import {
  buildCoverage,
  RESOURCE_PATH,
} from "../../scripts/presentation-resource-coverage.mjs";

const digest = (bytes) => createHash("sha256").update(bytes).digest("hex");
const temporaryRoot = fileURLToPath(
  new URL("../../.real-eval/", import.meta.url),
);
const repository = fileURLToPath(new URL("../../", import.meta.url));

for (const failure of ["mid-batch", "before-commit"])
  test(`a ${failure} failure preserves the previous bundle and permits a clean rerun`, async () => {
    await mkdir(temporaryRoot, { recursive: true });
    const base = await mkdtemp(
      path.join(temporaryRoot, "source-publish-regression-"),
    );
    try {
      const old = await readFile(
        path.join(
          repository,
          RESOURCE_PATH,
          "source-previews/blue-professional/page-01.jpg",
        ),
      );
      const page = "source-previews/cartesian/page-01.jpg";
      await mkdir(path.dirname(path.join(base, page)), { recursive: true });
      await writeFile(path.join(base, page), old);
      const evidence = {
        schemaVersion: 1,
        styles: [
          {
            id: "cartesian",
            pages: [{ number: 1, path: page, sha256: digest(old) }],
          },
        ],
      };
      const previous = JSON.stringify(evidence, null, 2) + "\n";
      await writeFile(
        path.join(base, "source-preview-evidence.json"),
        previous,
      );
      const newImage = await readFile(
        path.join(
          repository,
          RESOURCE_PATH,
          "source-previews/cartesian/page-01.jpg",
        ),
      );
      const files = new Map([
        [page, newImage],
        ["source-previews/cartesian/page-02.jpg", old],
      ]);
      const record = {
        id: "cartesian",
        pages: [...files].map(([relative, bytes], index) => ({
          number: index + 1,
          path: relative,
          sha256: digest(bytes),
          width: 1920,
          height: 1080,
        })),
      };
      const interrupt = () => {
        throw new Error("simulated batch interruption");
      };
      await assert.rejects(
        publishSourcePreviewBatch(
          base,
          [record],
          files,
          failure === "mid-batch"
            ? {
                beforePublishFile(index) {
                  if (index === 1) interrupt();
                },
              }
            : { beforeCommit: interrupt },
        ),
        /simulated batch interruption/,
      );
      assert.equal(
        digest(await readFile(path.join(base, page))),
        digest(old),
        "A failed batch must not overwrite previously evidenced source bytes",
      );
      assert.equal(
        await readFile(path.join(base, "source-preview-evidence.json"), "utf8"),
        previous,
      );
      await publishSourcePreviewBatch(base, [record], files);
      const published = JSON.parse(
        await readFile(path.join(base, "source-preview-evidence.json"), "utf8"),
      );
      assert.equal(published.styles.length, 1);
      assert.equal(published.styles[0].pages.length, 2);
      for (const item of published.styles[0].pages)
        assert.equal(
          digest(await readFile(path.join(base, item.path))),
          item.sha256,
        );
      assert.equal(
        digest(await readFile(path.join(base, page))),
        digest(old),
        "Successful publication also retains old evidenced assets",
      );
    } finally {
      await rm(base, { recursive: true, force: true });
    }
  });

test("source-only verification can start a genuine bundle repair when an old preview is corrupt", async () => {
  const fixture = await mkdtemp(
    path.join(temporaryRoot, "source-repair-regression-"),
  );
  try {
    const source = path.join(repository, RESOURCE_PATH),
      base = path.join(fixture, RESOURCE_PATH);
    await cp(source, base, { recursive: true });
    await mkdir(path.join(fixture, "assistant-runtime/src/python"), {
      recursive: true,
    });
    await cp(
      path.join(repository, "assistant-runtime/src/python/template.pptx"),
      path.join(fixture, "assistant-runtime/src/python/template.pptx"),
    );
    const evidence = JSON.parse(
      await readFile(path.join(base, "source-preview-evidence.json"), "utf8"),
    );
    const record = evidence.styles.find((entry) => entry.id === "cartesian");
    await writeFile(
      path.join(base, record.pages[0].path),
      "interrupted old preview",
    );
    await assert.rejects(buildCoverage(fixture), /hash mismatch/);
    assert.equal(
      (await buildCoverage(fixture, { sourceOnly: true })).originalFileCount,
      143,
    );
    const files = new Map();
    for (const page of record.pages) {
      const bytes = await readFile(path.join(source, page.path));
      page.path = `source-previews/cartesian/page-${String(page.number).padStart(2, "0")}.jpg`;
      files.set(page.path, bytes);
    }
    await publishSourcePreviewBatch(base, [record], files);
    const repaired = await buildCoverage(fixture);
    assert.equal(
      repaired.styles.find((entry) => entry.id === "cartesian").sourceDeck
        .preview.pages.length,
      10,
    );
  } finally {
    await rm(fixture, { recursive: true, force: true });
  }
});
