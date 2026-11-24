import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import path from "node:path";
import { createRequire } from "node:module";
import test from "node:test";
import {
  buildCampaign,
  newLedger,
  reserve,
  settle,
  validateReceipt,
  summary,
  prepare,
  withLedgerLock,
  forwarder,
  classifyWindows,
  boundedZip,
  assertCapturedPlan,
} from "../../scripts/assistant-parity-eval.mjs";

test("capture comparison accepts only the explicit process alias without changing raw evidence", () => {
  const input = {
    requirements: { slideCount: 1 },
    slides: [{ id: "page-1", relationship: "process", claim: "raw" }],
  };
  const before = structuredClone(input);
  const frozen = structuredClone(input);
  frozen.slides[0].relationship = "sequence";
  assert.doesNotThrow(() => assertCapturedPlan(input, frozen));
  assert.deepEqual(input, before);
  for (const wrong of ["spatial", "sequential", "Process"]) {
    const invalid = structuredClone(input);
    invalid.slides[0].relationship = wrong;
    assert.throws(() => assertCapturedPlan(invalid, frozen), /captured pages/);
  }
  const changed = structuredClone(frozen);
  changed.slides[0].claim = "changed";
  assert.throws(() => assertCapturedPlan(input, changed), /captured pages/);
  changed.requirements.slideCount = 2;
  assert.throws(
    () => assertCapturedPlan(input, changed),
    /captured requirements/,
  );
});

test("parity matrix fixes twelve tasks and keeps Win10 engine replay separate", () => {
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  assert.equal(campaign.tasks.length, 12);
  assert.deepEqual(campaign.models, [
    "deepseek/deepseek-v4.1-flash-1017",
    "openai/gpt-5-mini",
  ]);
  assert.equal(new Set(campaign.tasks.map((task) => task.id)).size, 12);
  assert.ok(campaign.tasks.every((task) => task.expectedSlides === 8));
  assert.equal(campaign.win10Replays.length, 6);
  assert.ok(
    campaign.win10Replays.every((task) => task.capturedPlatform === "win11"),
  );
  assert.equal(summary(campaign, []).status, "pending");
});

test("one shared ledger reserves task, allocation and campaign bounds before forwarding", () => {
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const ledger = newLedger(campaign);
  const task = campaign.tasks[0];
  reserve(ledger, task, "request-1", "0.60");
  assert.throws(
    () => reserve(ledger, task, "request-2", "0.41"),
    /task budget/,
  );
  settle(ledger, "request-1", "0.10");
  reserve(ledger, task, "request-2", "0.90");
  assert.throws(
    () => reserve(ledger, task, "request-3", "0.01"),
    /task budget/,
  );
  assert.throws(() => reserve(ledger, task, "request-2", "0.01"), /duplicate/);
});

test("missing usage or a failed submitted request keeps its reservation and stops", () => {
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const ledger = newLedger(campaign);
  reserve(ledger, campaign.tasks[0], "unknown", "0.50");
  settle(ledger, "unknown", null);
  assert.equal(ledger.stopped, "unknown_cost");
  assert.equal(ledger.requests[0].reservedUsd, "0.50");
  assert.equal(ledger.requests[0].costUsd, null);
  assert.throws(
    () => reserve(ledger, campaign.tasks[1], "later", "0.10"),
    /stopped/,
  );
});

test("successful exit and self-reported page counts do not make a strict receipt", () => {
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const task = campaign.tasks[0];
  assert.throws(
    () => validateReceipt(campaign, task, { status: "completed", exitCode: 0 }),
    /identity/,
  );
  assert.throws(
    () =>
      validateReceipt(campaign, task, {
        taskId: task.id,
        modelId: task.modelId,
        inputHash: task.inputHash,
        gitSha: campaign.gitSha,
        generationHash: campaign.generationHash,
        compiledSourceSha: campaign.gitSha,
        compiledSourceClean: true,
        nativeHost: { platform: task.platform },
        mode: "live",
        status: "completed",
        artifact: { slides: 8 },
      }),
    /artifact/,
  );
});

test("ledger lock refuses concurrent controllers and releases on a failed operation", async (t) => {
  const out = await mkdtemp(path.join(tmpdir(), "ofox-parity-lock-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  await withLedgerLock(out, async () => {
    await assert.rejects(
      withLedgerLock(out, async () => {}),
      /locked/,
    );
  });
  await assert.rejects(
    withLedgerLock(out, async () => {
      throw new Error("fixture");
    }),
    /fixture/,
  );
  await withLedgerLock(out, async () => {});
});

test("offline preparation writes pending tasks without reading a credential or making a request", async (t) => {
  const out = await mkdtemp(path.join(tmpdir(), "ofox-parity-offline-"));
  t.after(() => rm(out, { recursive: true, force: true }));
  await prepare(out);
  const campaign = JSON.parse(
    await readFile(path.join(out, "campaign.json"), "utf8"),
  );
  assert.equal(campaign.tasks.length, 12);
  assert.equal(campaign.realModelExecuted, false);
  assert.equal(summary(campaign, []).status, "pending");
  assert.equal(
    (await readFile(path.join(out, "business-source.pptx")))
      .subarray(0, 2)
      .toString(),
    "PK",
  );
  await assert.rejects(prepare(out), /already exists/);
});

test("guarded forwarding durably reserves first and keeps the real key out of the Host", async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), "ofox-parity-proxy-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const ledger = newLedger(campaign);
  const key = "synthetic-dedicated-parity-key";
  let forwarded = 0;
  const proxy = await forwarder({
    campaign,
    ledger,
    directory,
    key,
    apex: "ofox.io",
    catalog: {
      data: [
        {
          id: campaign.models[0],
          context_length: 1000,
          pricing: { prompt: "0.0000001", completion: "0.000001" },
        },
      ],
    },
    fetchImpl: async (url, options) => {
      forwarded++;
      assert.equal(url, "https://api.ofox.io/v1/chat/completions");
      const persisted = JSON.parse(
        await readFile(path.join(directory, "ledger.json"), "utf8"),
      );
      assert.equal(persisted.requests[0].status, "reserved");
      assert.equal(options.headers.Authorization, `Bearer ${key}`);
      assert.equal(JSON.parse(options.body).stream_options.include_usage, true);
      return new Response(
        `data: ${JSON.stringify({ choices: [{ delta: { content: key } }], usage: { prompt_tokens: 100, completion_tokens: 20 } })}\n\ndata: [DONE]\n\n`,
        { headers: { "content-type": "text/event-stream" } },
      );
    },
  });
  t.after(() => proxy.close());
  proxy.select(campaign.tasks[0]);
  const result = await fetch(`${proxy.base}/v1/chat/completions`, {
    method: "POST",
    headers: { Authorization: `Bearer ${proxy.token}` },
    body: JSON.stringify({
      model: campaign.models[0],
      stream: true,
      max_tokens: 100,
    }),
  });
  assert.equal(result.status, 200);
  assert.ok(!(await result.text()).includes(key));
  assert.equal(forwarded, 1);
  assert.equal(ledger.requests[0].costUsd, "0.00003");
  assert.equal(ledger.requests[0].status, "known");
});

test("a submitted response without usage halts later paid requests with its reservation retained", async (t) => {
  const directory = await mkdtemp(path.join(tmpdir(), "ofox-parity-unknown-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const ledger = newLedger(campaign);
  let forwarded = 0;
  const proxy = await forwarder({
    ledger,
    directory,
    key: "synthetic-private-key",
    apex: "ofox.ai",
    catalog: {
      data: [
        {
          id: campaign.models[0],
          context_length: 1000,
          pricing: { prompt: "0.0000001", completion: "0.000001" },
        },
      ],
    },
    fetchImpl: async () => {
      forwarded++;
      return new Response("data: [DONE]\n\n");
    },
  });
  t.after(() => proxy.close());
  proxy.select(campaign.tasks[0]);
  const send = () =>
    fetch(`${proxy.base}/v1/chat/completions`, {
      method: "POST",
      headers: { Authorization: `Bearer ${proxy.token}` },
      body: JSON.stringify({
        model: campaign.models[0],
        stream: true,
        max_tokens: 100,
      }),
    });
  assert.equal((await send()).status, 403);
  assert.equal(ledger.stopped, "unknown_cost");
  assert.equal(ledger.requests[0].status, "unknown");
  assert.equal(ledger.requests[0].costUsd, null);
  assert.equal((await send()).status, 403);
  assert.equal(forwarded, 1);
});

test("strict gate tolerates no failed task and requires all six Win10 replays", () => {
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const receipts = campaign.tasks.map((task) => {
    const artifactSha256 = "c".repeat(64);
    const evidence = { path: "review.txt", sha256: "d".repeat(64) };
    return {
      taskId: task.id,
      modelId: task.modelId,
      inputHash: task.inputHash,
      gitSha: campaign.gitSha,
      generationHash: campaign.generationHash,
      compiledSourceSha: campaign.gitSha,
      compiledSourceClean: true,
      nativeHost: {
        platform: task.platform,
        osRelease: task.platform === "win11" ? "10.0.26100" : "26.0.0",
        productType: "WinNT",
        kernelProductType: 1,
      },
      mode: "live",
      status: "completed",
      generatedAttachments: 1,
      actualSlides: 8,
      artifact: { path: "artifact.pptx", sha256: artifactSha256 },
      verifiedArtifact: {
        slides: 8,
        sha256: artifactSha256,
        nativeEquation: true,
        nativeChart: true,
        embeddedWorkbook: true,
        onlyPage3Changed: true,
      },
      route: "python",
      engineExecuted: true,
      isolationVerified: true,
      isolationReceipts: [
        {
          policy: "lpac-v1",
          trustedProbe: false,
          tokenVerified: true,
          childProcessDenied: true,
          lowImageDenied: true,
          jobStopped: true,
          cleanupComplete: true,
          taskRemoved: true,
          profileRemoved: true,
          cancelled: false,
          status: 0,
          executable: "node/node.exe",
          sequence: 1,
          runId: "synthetic-run",
          taskId: "synthetic-task",
          runtimeSha256: "f".repeat(64),
        },
      ],
      captureSha256: "e".repeat(64),
      frozenPlan: {
        planId: "plan",
        revision: 1,
        source: { version: task.sourceSha, pageNumbers: [3] },
      },
      checks: { validation: true, renderedPages: 8 },
      sourceSha: task.sourceSha,
      parentRevision: { parentVersion: task.sourceSha, pageNumbers: [3] },
      visualReview: {
        status: "passed",
        artifactSha256,
        reviewedPages: [1, 2, 3, 4, 5, 6, 7, 8],
        evidence,
      },
      targetApplicationReview: {
        status: "passed",
        artifactSha256,
        application: "Fixture",
        version: "1",
        savedAndReopened: true,
        evidence,
      },
    };
  });
  assert.equal(summary(campaign, receipts).status, "pending");
  const replays = campaign.win10Replays.map((item) => ({
    replayId: item.id,
    accepted: true,
  }));
  assert.equal(
    summary(campaign, receipts, replays).status,
    "pending",
    "Win10 client acceptance is still missing",
  );
  for (const replay of replays) replay.manualAccepted = true;
  const ledger = newLedger(campaign);
  for (const task of campaign.tasks) {
    reserve(ledger, task, task.id, "0.10");
    settle(ledger, task.id, "0.01");
    ledger.tasks[task.id] = { status: "completed", inputHash: task.inputHash };
  }
  assert.equal(summary(campaign, receipts, replays, ledger).status, "passed");
  ledger.stopped = "unknown_cost";
  assert.equal(
    summary(campaign, receipts, replays, ledger).status,
    "pending",
    "a stopped budget cannot pass from artifact receipts alone",
  );
  ledger.stopped = null;
  assert.equal(
    summary(
      campaign,
      [
        {
          ...receipts[0],
          compiledSourceSha: "f".repeat(40),
          compiledSourceClean: true,
        },
        ...receipts.slice(1),
      ],
      replays,
    ).status,
    "failed",
    "a caller SHA is not compiled-source proof",
  );
  assert.equal(
    summary(
      campaign,
      [{ ...receipts[0], status: "error" }, ...receipts.slice(1)],
      replays,
    ).status,
    "failed",
  );
  assert.equal(
    summary(
      campaign,
      [{ ...receipts[0], inputHash: "f".repeat(64) }, ...receipts.slice(1)],
      replays,
    ).status,
    "failed",
  );
});

test("native Windows acceptance requires a client product, not just a server-compatible build", () => {
  assert.equal(classifyWindows("10.0.19045", "WinNT"), "win10");
  assert.equal(classifyWindows("10.0.26100", "WinNT"), "win11");
  for (const product of ["ServerNT", "LanmanNT", undefined]) {
    assert.equal(classifyWindows("10.0.20348", product), "unsupported");
    assert.equal(classifyWindows("10.0.26100", product), "unsupported");
  }
});

test("a catalog lookup cannot release another request's exclusive forwarding slot", async (t) => {
  const directory = await mkdtemp(
    path.join(tmpdir(), "ofox-parity-concurrency-"),
  );
  t.after(() => rm(directory, { recursive: true, force: true }));
  const campaign = buildCampaign({
    sourceSha: "a".repeat(64),
    gitSha: "b".repeat(40),
  });
  const ledger = newLedger(campaign);
  let release, announce;
  const gate = new Promise((resolve) => {
    release = resolve;
  });
  const started = new Promise((resolve) => {
    announce = resolve;
  });
  let forwarded = 0;
  const proxy = await forwarder({
    ledger,
    directory,
    key: "synthetic-private-key",
    apex: "ofox.ai",
    catalog: {
      data: [
        {
          id: campaign.models[0],
          context_length: 1000,
          pricing: { prompt: "0.0000001", completion: "0.000001" },
        },
      ],
    },
    fetchImpl: async () => {
      forwarded++;
      announce();
      await gate;
      return new Response(
        'data: {"usage":{"prompt_tokens":10,"completion_tokens":5}}\n\ndata: [DONE]\n\n',
      );
    },
  });
  t.after(() => {
    release();
    return proxy.close();
  });
  proxy.select(campaign.tasks[0]);
  const headers = { Authorization: `Bearer ${proxy.token}` };
  const send = () =>
    fetch(`${proxy.base}/v1/chat/completions`, {
      method: "POST",
      headers,
      body: JSON.stringify({
        model: campaign.models[0],
        stream: true,
        max_tokens: 100,
      }),
    });
  const first = send();
  await started;
  assert.equal(
    (await fetch(`${proxy.base}/v1/models`, { headers })).status,
    200,
  );
  assert.equal((await send()).status, 403);
  assert.equal(forwarded, 1);
  release();
  assert.equal((await first).status, 200);
  assert.equal(ledger.requests.length, 1);
});

test("ZIP inspection rejects declared expansion before starting any inflation", async () => {
  const require = createRequire(import.meta.resolve("pptxgenjs"));
  const JSZip = require("jszip"),
    compression = require("jszip/lib/compressions");
  const zip = new JSZip();
  zip.file("tiny.txt", "tiny synthetic fixture");
  const bytes = await zip.generateAsync({
    type: "nodebuffer",
    compression: "DEFLATE",
  });
  const valid = await boundedZip(bytes);
  assert.equal(
    await valid.file("tiny.txt").async("string"),
    "tiny synthetic fixture",
  );
  valid.close();
  const central = bytes.indexOf(Buffer.from([0x50, 0x4b, 0x01, 0x02]));
  assert.ok(central >= 0);
  bytes.writeUInt32LE(33 * 1024 * 1024, central + 24);
  let inflated = 0;
  const original = compression.DEFLATE.uncompressWorker;
  compression.DEFLATE.uncompressWorker = function (...args) {
    inflated++;
    return original.apply(this, args);
  };
  try {
    await assert.rejects(boundedZip(bytes), /archive inspection bound/);
  } finally {
    compression.DEFLATE.uncompressWorker = original;
  }
  assert.equal(inflated, 0, "metadata refusal must precede CRC/decompression");
});
