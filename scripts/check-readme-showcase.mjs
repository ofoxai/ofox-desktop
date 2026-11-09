/** Dependency-free README/media preflight. Does not contact external URLs. */
import assert from "node:assert/strict";
import { existsSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const documents = ["README.md", "README.zh-CN.md", "README.ja.md"];
let anchors;
let external;
for (const file of documents) {
  const text = readFileSync(path.join(root, file), "utf8");
  const ids = [...text.matchAll(/<a id="([^"]+)"/g)].map((m) => m[1]);
  if (anchors) assert.deepEqual(ids, anchors, `${file}: section parity`);
  anchors = ids;
  const urls = [
    ...new Set(
      [...text.matchAll(/\]\((https?:[^)]+)\)|href="(https?:[^"]+)"/g)].map(
        (m) => m[1] ?? m[2],
      ),
    ),
  ]
    .map((value) => {
      const url = new URL(value);
      if (url.hostname === "ofox.ai")
        url.pathname = url.pathname.replace(/^\/(en|zh|ja)(?=\/|$)/, "") || "/";
      return url.href;
    })
    .sort();
  if (external)
    assert.deepEqual(urls, external, `${file}: external-link parity`);
  external = urls;
  const refs = [
    ...text.matchAll(/\]\(([^)]+)\)|(?:src|srcset|href)="([^"]+)"/g),
  ].map((m) => m[1] ?? m[2]);
  for (const ref of refs) {
    if (/^(https?:|mailto:)/.test(ref)) continue;
    const [relative, fragment] = ref.split("#");
    if (!relative) {
      assert(ids.includes(fragment), `${file}: missing anchor ${fragment}`);
      continue;
    }
    assert(
      existsSync(path.resolve(root, relative)),
      `${file}: missing ${relative}`,
    );
  }
  assert.equal(
    (text.match(/<details>/g) ?? []).length,
    6,
    `${file}: detail parity`,
  );
  assert.equal(
    (
      text.match(
        /<img\s+src="assets\/readme\/(?:sessions|library|discover|usage)\.webp"[^>]+width="1440"/g,
      ) ?? []
    ).length,
    4,
    `${file}: full-width gallery parity`,
  );
  const pictures = [...text.matchAll(/<picture>([\s\S]*?)<\/picture>/g)];
  assert.equal(pictures.length, 16, `${file}: themed media count`);
  assert.deepEqual(
    [...text.matchAll(/<img\s+src="([^"]+\.gif)"/g)].map((match) => match[1]),
    ["assets/readme/onboarding.gif"],
    `${file}: show the primary GIF exactly once`,
  );
  const matrixStart = text.indexOf(
    "\n|",
    text.indexOf('<a id="compatibility">'),
  );
  assert(matrixStart >= 0, `${file}: missing compatibility matrix`);
  const foldsBeforeMatrix = [
    ...text.slice(0, matrixStart).matchAll(/<\/?details>/g),
  ].reduce((depth, match) => depth + (match[0] === "<details>" ? 1 : -1), 0);
  assert.equal(
    foldsBeforeMatrix,
    0,
    `${file}: matrix must be visible by default`,
  );
  for (const [, picture] of pictures) {
    const light = picture.match(
      /<source media="\(prefers-color-scheme: light\)" srcset="([^"]+)"/,
    )?.[1];
    const dark = picture.match(/<img\s+src="([^"]+)"/)?.[1];
    assert(dark && light, `${file}: missing theme source or fallback`);
    assert.equal(
      light,
      dark.replace(/(\.[^.]+)$/, "-light$1"),
      `${file}: theme pair mismatch`,
    );
  }
}
const assets = path.join(root, "assets/readme");
const darkAssets = [
  "hero.svg",
  "hero-preview.webp",
  "tool-wall.svg",
  "data-flow.svg",
  "tour.gif",
  "tour-poster.webp",
  "tour.mp4",
  "tools-en.webp",
  "tools-zh.webp",
  "tools-ja.webp",
  "sessions.webp",
  "library.webp",
  "discover.webp",
  "assistant.webp",
  "assistant-media.svg",
  "assistant-image.webp",
  "assistant-video.webp",
  "assistant-speech.webp",
  "skills.webp",
  "usage.webp",
  "binding-flow.svg",
  "configuration-rescue.svg",
  "onboarding.gif",
  "onboarding-poster.webp",
  "onboarding.mp4",
];
const expected = [
  ...darkAssets,
  ...darkAssets.map((name) => name.replace(/(\.[^.]+)$/, "-light$1")),
];
for (const name of expected) {
  const file = path.join(assets, name);
  assert(existsSync(file), `missing media: ${name}`);
  const max = name.endsWith(".mp4")
    ? 12 * 1024 * 1024
    : name.endsWith(".gif")
      ? 8 * 1024 * 1024
      : 1024 * 1024;
  assert(statSync(file).size <= max, `media exceeds size budget: ${name}`);
  if (name.endsWith(".svg")) {
    const xml = readFileSync(file, "utf8");
    assert(
      !/<script\b|<foreignObject\b|onload=|href="https?:/i.test(xml),
      `${name}: active/external SVG content`,
    );
  } else {
    const probe = spawnSync(
      "ffprobe",
      [
        "-v",
        "error",
        "-show_entries",
        "stream=width,height,nb_frames,codec_name:format=duration",
        "-of",
        "json",
        file,
      ],
      { encoding: "utf8" },
    );
    assert.equal(probe.status, 0, probe.stderr);
    const info = JSON.parse(probe.stdout);
    const expectedSize = name.startsWith("hero-preview")
      ? [3200, 1820]
      : name.endsWith(".mp4")
        ? [1440, 900]
        : name.endsWith(".gif")
          ? [1200, 750]
          : [2880, 1800];
    assert.deepEqual(
      [info.streams[0].width, info.streams[0].height],
      expectedSize,
      `${name}: dimensions`,
    );
    if (name.endsWith(".gif")) {
      const expectedDuration = name.startsWith("onboarding") ? 6.8 : 8;
      assert(
        Math.abs(Number(info.format.duration) - expectedDuration) < 0.1,
        `${name}: compact animation duration`,
      );
      assert(
        Math.abs(Number(info.streams[0].nb_frames) - expectedDuration * 12) <=
          1,
        `${name}: frame count`,
      );
    } else if (name.endsWith(".mp4")) {
      assert.equal(
        info.streams[0].codec_name,
        "h264",
        `${name}: browser-compatible codec`,
      );
      assert(
        Math.abs(
          Number(info.format.duration) -
            (name.startsWith("onboarding") ? 6.8 : 8),
        ) < 0.05,
        `${name}: video duration`,
      );
      assert.equal(info.streams.length, 1, `${name}: no captured audio`);
    } else {
      const bytes = readFileSync(file);
      for (let offset = 12; offset + 8 <= bytes.length; ) {
        const chunk = bytes.toString("ascii", offset, offset + 4);
        assert(
          !["EXIF", "XMP ", "ICCP"].includes(chunk),
          `${name}: embedded metadata`,
        );
        const size = bytes.readUInt32LE(offset + 4);
        offset += 8 + size + (size % 2);
      }
    }
  }
}
const xmlCheck = spawnSync(
  "python3",
  [
    "-c",
    "import sys,xml.etree.ElementTree as E; [E.parse(p) for p in sys.argv[1:]]",
    ...expected
      .filter((x) => x.endsWith(".svg"))
      .map((x) => path.join(assets, x)),
  ],
  { encoding: "utf8" },
);
assert.equal(xmlCheck.status, 0, xmlCheck.stderr);
console.log(
  `PASS: ${documents.length} languages, ${anchors.length} section anchors, ${expected.length} media files; links, XML and size budgets valid.`,
);
