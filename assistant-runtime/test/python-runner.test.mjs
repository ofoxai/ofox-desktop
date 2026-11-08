import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  access,
  copyFile,
  mkdir,
  mkdtemp,
  readFile,
  readdir,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { buffer } from "node:stream/consumers";
import { fileURLToPath } from "node:url";
import { promisify } from "node:util";
import yauzl from "yauzl";

// The runner is driven the way the host will drive it: the staged Node, a
// code file, and the staged Python engine assets. Where the engine is offered
// (macOS) a missing asset fails the test; elsewhere the test cannot run.
const root = fileURLToPath(new URL("../", import.meta.url));
const staged = path.join(root, "resources/node/node");
const assets = path.join(root, "resources/engines/python");
const runner = path.join(root, "src/python/runner.mjs");
const skip =
  process.platform === "darwin" ? false : "the Python engine is macOS-only";
// A cold Pyodide start (numpy, matplotlib, lxml) takes several seconds.
const options = { skip, timeout: 180_000 };

const exists = (file) =>
  access(file).then(
    () => true,
    () => false,
  );

async function requireEngine() {
  for (const file of [staged, path.join(assets, "assets.json")])
    if (!(await exists(file)))
      assert.fail(
        `${path.relative(root, file)} is missing: run pnpm assistant:prepare first`,
      );
}

async function scratch(t) {
  const directory = await mkdtemp(path.join(tmpdir(), "ofox-python-runner-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  return directory;
}

/** Runs `code` in the runner and returns its exit status, stderr and files. */
async function run(t, code, { assetsDir = assets } = {}) {
  await requireEngine();
  const directory = await scratch(t);
  const files = {
    code: path.join(directory, "code.py"),
    out: path.join(directory, "out.pptx"),
    lint: path.join(directory, "lint.json"),
  };
  await writeFile(files.code, code);
  const child = spawn(
    staged,
    [
      runner,
      "--assets",
      assetsDir,
      "--code",
      files.code,
      "--out",
      files.out,
      "--lint",
      files.lint,
    ],
    {
      env: { PATH: process.env.PATH },
      stdio: ["ignore", "pipe", "pipe"],
      signal: AbortSignal.timeout(150_000),
    },
  );
  const stderr = buffer(child.stderr).then((bytes) => bytes.toString("utf8"));
  buffer(child.stdout).catch(() => {});
  const status = await new Promise((resolve, reject) => {
    child.on("error", reject);
    child.on("close", resolve);
  });
  return { status, stderr: await stderr, ...files };
}

/** Every entry of a zip file by name. */
async function zipEntries(file) {
  const zip = await promisify(yauzl.open)(file, { lazyEntries: true });
  const open = promisify(zip.openReadStream.bind(zip));
  const entries = new Map();
  await new Promise((resolve, reject) => {
    zip.on("error", reject);
    zip.on("end", resolve);
    zip.on("entry", async (entry) => {
      try {
        entries.set(entry.fileName, await buffer(await open(entry)));
        zip.readEntry();
      } catch (error) {
        reject(error);
      }
    });
    zip.readEntry();
  });
  return entries;
}

const TITLE_SLIDE = `
prs = new_deck(title="t")
content_slide(prs, "Hello")
`;

test("makes a pptx from code", options, async (t) => {
  const result = await run(t, TITLE_SLIDE);
  assert.equal(result.status, 0, result.stderr);
  const entries = await zipEntries(result.out);
  assert.ok(entries.has("ppt/slides/slide1.xml"));
  const lint = JSON.parse(await readFile(result.lint, "utf8"));
  assert.ok(Array.isArray(lint));
});

test("reports a Python error", options, async (t) => {
  const result = await run(t, "prs = new_deck()\nprint(undefined_name)\n");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /^PythonError: /);
  assert.match(result.stderr, /NameError/);
  // The traceback is cut to its end, not left to fill the host's buffer.
  assert.ok(result.stderr.length <= "PythonError: ".length + 1800 + 1);
  assert.equal(await exists(result.out), false);
});

test("requires prs", options, async (t) => {
  const result = await run(t, "x = 1\n");
  assert.equal(result.status, 1);
  assert.match(result.stderr, /the code must create prs = new_deck\(\.\.\.\)/);
});

test("writes non-negative chart axis ids", options, async (t) => {
  const result = await run(
    t,
    `
prs = new_deck(title="t")
slide = content_slide(prs, "Chart")
data = CategoryChartData()
data.categories = ["A", "B", "C"]
data.add_series("Sales", (1, 2, 3))
slide.shapes.add_chart(
    XL_CHART_TYPE.COLUMN_CLUSTERED, Inches(1), Inches(2), Inches(6), Inches(4), data
)
`,
  );
  assert.equal(result.status, 0, result.stderr);
  const entries = await zipEntries(result.out);
  const charts = [...entries.keys()].filter((name) =>
    /^ppt\/charts\/chart\d+\.xml$/.test(name),
  );
  assert.equal(charts.length, 1);
  for (const name of charts) {
    const xml = entries.get(name).toString("utf8");
    for (const [, tag, value] of xml.matchAll(
      /<c:(axId|crossAx) val="([^"]*)"/g,
    ))
      assert.match(value, /^\d+$/, `${name} ${tag}`);
    // The category and value axes still point at each other, and the plot
    // still names exactly those two axes.
    const axes = [...xml.matchAll(/<c:(catAx|valAx)>([\s\S]*?)<\/c:\1>/g)].map(
      ([, , body]) => ({
        id: body.match(/<c:axId val="(\d+)"/)[1],
        cross: body.match(/<c:crossAx val="(\d+)"/)[1],
      }),
    );
    assert.equal(axes.length, 2, name);
    assert.equal(axes[0].cross, axes[1].id, name);
    assert.equal(axes[1].cross, axes[0].id, name);
    assert.notEqual(axes[0].id, axes[1].id, name);
    const plotted = [
      ...xml
        .match(/<c:barChart>[\s\S]*?<\/c:barChart>/)[0]
        .matchAll(/<c:axId val="(\d+)"/g),
    ].map(([, id]) => id);
    assert.deepEqual(plotted.sort(), axes.map((axis) => axis.id).sort(), name);
  }
});

test("fails on a missing wheel", options, async (t) => {
  await requireEngine();
  const manifest = JSON.parse(
    await readFile(path.join(assets, "assets.json"), "utf8"),
  );
  const numpy = (await readdir(path.join(assets, "wheels"))).find((file) =>
    file.startsWith("numpy-"),
  );
  // One pure-Python wheel from the manifest and one Pyodide wheel named by
  // the lock file: each is missing from its own copy of the assets.
  for (const missing of [manifest.pure[0], numpy]) {
    const copy = await scratch(t);
    await mkdir(path.join(copy, "wheels"));
    for (const file of await readdir(path.join(assets, "wheels")))
      if (file !== missing)
        await symlink(
          path.join(assets, "wheels", file),
          path.join(copy, "wheels", file),
        );
    await symlink(path.join(assets, "fonts"), path.join(copy, "fonts"));
    await copyFile(
      path.join(assets, "assets.json"),
      path.join(copy, "assets.json"),
    );
    const result = await run(t, TITLE_SLIDE, { assetsDir: copy });
    assert.equal(result.status, 1, missing);
    assert.match(result.stderr, /RunnerError/, missing);
    assert.ok(result.stderr.includes(missing), result.stderr);
    assert.equal(await exists(result.out), false, missing);
  }
});

test("draws an SVG figure", options, async (t) => {
  const result = await run(
    t,
    `
prs = new_deck(title="t")
slide = content_slide(prs, "Figure")
figure(
    slide,
    '<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300">'
    '<rect width="400" height="300" fill="#1E3A5F"/>'
    '<circle cx="200" cy="150" r="80" fill="#D9822B"/></svg>',
    1, 2, 4, 3,
)
`,
  );
  assert.equal(result.status, 0, result.stderr);
  const entries = await zipEntries(result.out);
  const images = [...entries.keys()].filter((name) =>
    /^ppt\/media\/.*\.png$/.test(name),
  );
  assert.equal(images.length, 1);
  assert.deepEqual(
    [...entries.get(images[0]).subarray(0, 4)],
    [0x89, 0x50, 0x4e, 0x47],
  );
});
