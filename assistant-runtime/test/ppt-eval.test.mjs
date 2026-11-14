import assert from "node:assert/strict";
import { execFile } from "node:child_process";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { promisify } from "node:util";
import { fileURLToPath } from "node:url";
import {
  benchmarkHash,
  buildExperiment,
  inspectPptx,
  loadBenchmark,
  mergeReceipts,
} from "../../scripts/assistant-ppt-eval.mjs";

const style = {
  id: "cartesian",
  version: "fixed-public-sha",
  status: "adapted",
};
const context = {
  model: "test/model",
  modelVersion: "2026-11-10",
  engine: "python",
  engineVersion: "sha256:helper",
  runtimeVersion: "git:test",
  platform: "test-os/arm64",
  parameters: { temperature: 0, seed: 7 },
};
const experiment = async () =>
  buildExperiment(await loadBenchmark(), {
    styles: [style],
    repeats: 3,
    context,
  });

test("benchmark is independently authored, six slides, with frozen data and limitations", async () => {
  const benchmark = await loadBenchmark();
  assert.equal(benchmark.slides.length, 6);
  assert.equal(benchmark.provenance.kind, "self-authored-synthetic");
  assert.deepEqual(
    benchmark.data.map((row) => [row.input, row.valid, row.seconds]),
    [
      [100, 90, 12],
      [200, 180, 24],
      [300, 270, 36],
    ],
  );
  assert.match(benchmark.slides[5].body.join(""), /不能.*B.*A/);
  assert.equal(benchmarkHash({ b: 2, a: 1 }), benchmarkHash({ a: 1, b: 2 }));
  assert.notEqual(
    benchmarkHash(benchmark),
    benchmarkHash({ ...benchmark, language: "en" }),
  );
});

test("four arms keep content and environment fixed without inventing results or weakening A", async () => {
  const plan = await experiment();
  assert.equal(plan.runs.length, 12);
  for (const run of plan.runs) {
    assert.equal(run.frozen.contentHash, plan.contentHash);
    assert.deepEqual(run.frozen.parameters, context.parameters);
    assert.equal(run.status, "pending");
    assert.equal(run.comparability, "blocked");
    assert.equal(run.preflight.status, "blocked");
    assert.match(run.preflight.reason, /shared guidelines/);
    assert.equal(run.manualReview.status, "pending");
    assert.equal(run.cost.actual, null);
    assert.equal(run.cost.estimate, null);
    assert.equal(run.metrics.tokens, null);
    assert.equal(run.policy.preserveExistingPreviewAndChecks, true);
  }
  const arms = Object.fromEntries(
    plan.runs.slice(0, 4).map((run) => [run.condition, run]),
  );
  assert.equal(arms.A.frozen.style.id, "desktop-current");
  assert.equal(arms.B.policy.newSlidePlan, false);
  assert.equal(arms.C.policy.newSlidePlan, true);
  assert.equal(arms.D.policy.additionalVisualFixLimit, 2);
  assert.equal(arms.B.frozen.style.version, arms.D.frozen.style.version);
});

test("receipts reject confounded arms and unsubstantiated billing; manual evidence remains separate", async () => {
  const plan = await experiment();
  const run = plan.runs[1];
  const receipt = {
    runId: run.id,
    frozen: run.frozen,
    status: "completed",
    artifacts: ["deck.pptx"],
    metrics: {
      tokens: { input: 100, output: 20 },
      seconds: { draft: 2, final: 3 },
    },
    cost: {
      estimate: {
        amount: 0.01,
        currency: "USD",
        priceVersion: "catalog:fixed",
        pricedAt: "2026-11-10",
      },
      actual: null,
    },
  };
  const result = mergeReceipts(plan, [receipt]);
  assert.equal(result.runs[1].status, "completed");
  assert.equal(result.runs[1].comparability, "blocked");
  assert.equal(result.runs[1].manualReview.status, "pending");
  assert.equal(result.runs[1].cost.actual, null);
  assert.throws(
    () =>
      mergeReceipts(plan, [
        { ...receipt, frozen: { ...run.frozen, contentHash: "changed" } },
      ]),
    /frozen/,
  );
  assert.throws(
    () =>
      mergeReceipts(plan, [
        { ...receipt, cost: { actual: { amount: 0.01, currency: "USD" } } },
      ]),
    /billing/,
  );
  assert.throws(() => mergeReceipts(plan, [receipt, receipt]), /duplicate/);
});

test("OOXML presence checks cannot claim native PowerPoint editing or visual quality", async () => {
  const benchmark = await loadBenchmark();
  const entries = new Map(
    benchmark.slides.map((slide, i) => [
      `ppt/slides/slide${i + 1}.xml`,
      Buffer.from(
        `<a:t>${slide.title}</a:t>${slide.requiredText.map((text) => `<a:t>${text}</a:t>`).join("")}`,
      ),
    ]),
  );
  entries.set(
    "ppt/presentation.xml",
    Buffer.from('<p:sldSz cx="12192000" cy="6858000"/>'),
  );
  const inspected = inspectPptx(entries, benchmark);
  assert.equal(inspected.slideCount, 6);
  assert.equal(inspected.nativeChart, false);
  assert.equal(inspected.nativeEquation, false);
  assert.equal(inspected.powerpointEditability, "pending");
  assert.equal(inspected.visualReview, "pending");
  assert.deepEqual(inspected.missingFacts, []);
});

test("offline CLI writes pending model runs and never needs credentials", async (t) => {
  const out = await mkdtemp(path.join(tmpdir(), "ofox-ppt-eval-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  await promisify(execFile)(
    process.execPath,
    [
      fileURLToPath(
        new URL("../../scripts/assistant-ppt-eval.mjs", import.meta.url),
      ),
      "--out",
      out,
      "--styles",
      "cartesian",
      "--repeats",
      "1",
    ],
    { env: { PATH: process.env.PATH } },
  );
  const plan = JSON.parse(
    await readFile(path.join(out, "experiment.json"), "utf8"),
  );
  assert.ok(
    plan.generationFiles[
      "src-tauri/src/services/assistant/documents/create.rs"
    ],
    "Snapshot must cover the actual Rust assistant implementation",
  );
  assert.equal(plan.runs.length, 4);
  assert.equal(plan.realModelExecuted, false);
  assert.match(plan.runs[0].frozen.generationSnapshotHash, /^[a-f0-9]{64}$/);
  assert.ok(plan.generationFiles["assistant-runtime/src/document-tools.mjs"]);
  assert.ok(
    plan.generationFiles["assistant-runtime/skills/ofox-office-pptx/SKILL.md"],
  );
  assert.ok(plan.runs.every((run) => run.status === "pending"));
  const prompt = await readFile(
    path.join(out, "prompts", plan.runs[0].id + ".md"),
    "utf8",
  );
  assert.match(prompt, /现有.*预览/);
  assert.match(prompt, /180.*200.*90%/);
});
