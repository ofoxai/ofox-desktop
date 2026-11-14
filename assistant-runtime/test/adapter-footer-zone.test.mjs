// Content pages carry a footer line at the bottom left. A style's own
// decoration must not sit on it (found in a real daisy-days deck, where a
// corner flower covered the footer and the tip strip above it).
import assert from "node:assert/strict";
import { readFile, readdir } from "node:fs/promises";
import test from "node:test";

const adapters = new URL(
  "../skills/ofox-office-pptx/resources/adapters/",
  import.meta.url,
);

test("no style decorates a content page over its footer line", async () => {
  for (const file of (await readdir(adapters)).filter((name) =>
    name.endsWith(".json"),
  )) {
    const adapter = JSON.parse(await readFile(new URL(file, adapters), "utf8"));
    const shapes = adapter.theme?.decor?.layout?.content?.shapes ?? [];
    const onFooter = shapes.filter(
      (shape) =>
        [shape.x, shape.y, shape.w, shape.h].every(Number.isFinite) &&
        // A small mark, not a panel or band that the footer is drawn on.
        shape.w < 4 &&
        shape.h < 3 &&
        shape.y > 5.5 &&
        shape.y + shape.h > 6.6 &&
        shape.x < 6.5,
    );
    assert.deepEqual(
      onFooter.map((shape) => shape.name),
      [],
      `${file}: decoration over the content footer`,
    );
  }
});
