// Rebuild only the reviewed, pinned six-slide public architectural demo.
// The upstream script runs from a new task directory, never from vendor or a
// user workspace. No dependency installation, remote request or user data input.
import { spawnSync } from "node:child_process";
import {
  mkdir,
  mkdtemp,
  readFile,
  writeFile,
  copyFile,
} from "node:fs/promises";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildCoverage,
  hashBytes,
  RESOURCE_PATH,
} from "./presentation-resource-coverage.mjs";
import { publishSourcePreviewBatch } from "./source-preview-publish.mjs";

const root = path.resolve(fileURLToPath(new URL("../", import.meta.url)));
const base = path.join(root, RESOURCE_PATH);
const runtime =
  "/Applications/Ofox Desktop.app/Contents/Resources/assistant-runtime";
const office = path.join(runtime, "engines/officecli/officecli");
const node = path.join(runtime, "node/node");
const coverage = await buildCoverage(root, { sourceOnly: true });
const style = coverage.styles.find(
  (entry) => entry.id === "architectural-plan",
);
const source = style.sourceDeck.files.find((file) =>
  file.path.endsWith("build.sh"),
);
const evidenceRoot = path.join(
  root,
  ".real-eval/ppt-source-coverage-2026-11-10",
);
await mkdir(evidenceRoot, { recursive: true });
const stage = await mkdtemp(path.join(evidenceRoot, "architectural-original-"));
await mkdir(path.join(stage, "bin"));
await mkdir(path.join(stage, "isolated-home"));
const quote = (value) => "'" + value.replaceAll("'", "'\\''") + "'";
await writeFile(
  path.join(stage, "bin/officecli"),
  `#!/bin/sh\nexec ${quote(office)} "$@"\n`,
  { mode: 0o700 },
);
await copyFile(path.join(base, source.path), path.join(stage, "build.sh"));
// HOME is scoped only to these child processes; the parent environment and all
// real user profiles remain untouched.
const environment = {
  ...process.env,
  HOME: path.join(stage, "isolated-home"),
  PATH: `${path.join(stage, "bin")}:/usr/bin:/bin:/usr/sbin:/sbin`,
  OFFICECLI_NO_AUTO_RESIDENT: "1",
};
function run(command, args, label) {
  const result = spawnSync(command, args, {
    cwd: stage,
    env: environment,
    encoding: "utf8",
    timeout: 240000,
    maxBuffer: 32 * 1024 * 1024,
  });
  if (result.error || result.status !== 0)
    throw new Error(
      `${label} failed: ${result.error?.message || result.stderr?.slice(-2000) || result.stdout?.slice(-2000)}`,
    );
  return result.stdout;
}
const officeVersion = run(office, ["--version"], "OfficeCLI version").trim();
const output = run(
  "/bin/bash",
  [path.join(stage, "build.sh")],
  "Pinned original build",
);
await writeFile(path.join(stage, "build.log"), output);
const pptx = path.join(stage, "dark__architectural_plan.pptx");
const validation = JSON.parse(
  run(office, ["validate", pptx, "--json"], "Original schema validation"),
);
if (validation.data?.errors?.length)
  throw new Error("Original source build has schema validation findings");
const pages = [];
const outputFiles = new Map();
const destination = path.join(base, "source-previews/architectural-plan");
await mkdir(destination, { recursive: true });
const renderCode = `import{readFile,writeFile}from'node:fs/promises';import{pathToFileURL}from'node:url';import path from'node:path';const [runtime,svg,out]=process.argv.slice(1);const{thumbnailFromSvg}=await import(pathToFileURL(path.join(runtime,'runtime/src/thumbnails.mjs')));const{loadImage}=await import(pathToFileURL(path.join(runtime,'runtime/node_modules/@napi-rs/canvas/index.js')));const image=await thumbnailFromSvg(await readFile(svg),{maxEdge:1920,maxBytes:512*1024});if(!image)throw Error('No JPEG');const bytes=Buffer.from(image.data,'base64');const decoded=await loadImage(bytes);if(decoded.width!==1920||decoded.height!==1080)throw Error('Unexpected dimensions');await writeFile(out,bytes);`;
for (const page of style.sourceDeck.pages) {
  const svg = JSON.parse(
    run(
      office,
      [
        "view",
        pptx,
        "svg",
        "--start",
        String(page.number),
        "--end",
        String(page.number),
        "--json",
      ],
      `Original SVG ${page.number}`,
    ),
  ).data;
  if (typeof svg !== "string")
    throw new Error("OfficeCLI did not return original-page SVG");
  const svgPath = path.join(stage, `page-${page.number}.svg`);
  await writeFile(svgPath, svg);
  const relative = `source-previews/architectural-plan/page-${String(page.number).padStart(2, "0")}.jpg`;
  const rasterPath = path.join(stage, `page-${page.number}.jpg`);
  run(
    node,
    ["--input-type=module", "-e", renderCode, runtime, svgPath, rasterPath],
    `Original JPEG ${page.number}`,
  );
  const raster = await readFile(rasterPath);
  outputFiles.set(relative, raster);
  pages.push({
    number: page.number,
    path: relative,
    sha256: hashBytes(raster),
    width: 1920,
    height: 1080,
  });
}
const sourceBytes = await readFile(pptx);
outputFiles.set(
  "source-previews/architectural-plan/rebuilt-original.pptx",
  sourceBytes,
);
const record = {
  id: style.id,
  status: "rendered-with-local-fonts",
  pageCount: 6,
  sourceSha256: source.sha256,
  inputFiles: style.sourceDeck.files.map(({ path: filePath, sha256 }) => ({
    path: filePath,
    sha256,
  })),
  renderer: {
    name: `OfficeCLI ${officeVersion}; installed signed Node/canvas`,
    executable: office,
    network:
      "Pinned script contains no remote resources; OfficeCLI auto-resident disabled, private child HOME, task-only output paths.",
    activation:
      "Actual public build.sh rebuilt all six slides; OfficeCLI SVG rendered directly, not from Ofox's independent sample.",
  },
  fontPolicy:
    "No explicit font family in original script; local engine/system fonts only, no font binary fetched or embedded.",
  rebuiltSourceDeck: {
    path: "source-previews/architectural-plan/rebuilt-original.pptx",
    sha256: hashBytes(sourceBytes),
    generatedFromPinnedSource: true,
  },
  schemaValidation: validation.data,
  pages,
  omittedPages: [],
  blockedUrls: [],
  runtimeErrors: [],
  visualReview: "pending",
  visualFindings: [
    {
      code: "original-card-text-clipped",
      severity: "warning",
      pages: [2, 4, 5],
      message:
        "Known original-source card text clipping observed with OfficeCLI 1.0.160; re-review after regeneration.",
    },
    {
      code: "original-footer-stat-overlap",
      severity: "warning",
      pages: [2, 3, 4, 5, 6],
      message:
        "Known original-source lower statistics/footer overlap or clipping; schema acceptance is not visual acceptance.",
    },
  ],
  nativeEditing: "not-inferred",
};
await publishSourcePreviewBatch(base, [record], outputFiles, {
  stagingRoot: evidenceRoot,
});
console.log(
  JSON.stringify({
    style: style.id,
    originalPages: 6,
    rendered: pages.length,
    officeVersion,
    sourcePptxSha256: hashBytes(sourceBytes),
  }),
);
