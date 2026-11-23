// Strict #1701 parity campaign. Default preparation is offline and key-free.
// Paid requests belong only to this controller; the Host gets a loopback token.
import assert from "node:assert/strict";
import { createHash, randomUUID } from "node:crypto";
import { createServer } from "node:http";
import { spawn, execFileSync } from "node:child_process";
import { createRequire } from "node:module";
import {
  mkdir,
  open,
  readFile,
  readdir,
  rename,
  rm,
  lstat,
} from "node:fs/promises";
import path from "node:path";
import os from "node:os";
import { fileURLToPath, pathToFileURL } from "node:url";
import { crc32 } from "node:zlib";
import { redact } from "../assistant-runtime/src/security.mjs";
import { normalizePresentationPlanInput } from "../assistant-runtime/src/presentation-plan-input.mjs";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const MODELS = ["deepseek/deepseek-v4.1-flash-1017", "openai/gpt-5-mini"];
const UNIT = 1_000_000_000_000n;
const hash = (value) => createHash("sha256").update(value).digest("hex");
const canonical = (value) =>
  JSON.stringify(value, (_, item) =>
    item && typeof item === "object" && !Array.isArray(item)
      ? Object.fromEntries(
          Object.keys(item)
            .sort()
            .map((key) => [key, item[key]]),
        )
      : item,
  );
const digest = (value) => hash(canonical(value));
const COMPILED_SOURCES = [
  "src-tauri/src/services/assistant/host.rs",
  "src-tauri/src/services/assistant/host/local.rs",
  "src-tauri/src/services/assistant/documents/create.rs",
  "src-tauri/src/services/assistant/documents/runner.rs",
  "src-tauri/src/services/assistant/documents/engines.rs",
  "src-tauri/src/services/assistant/documents/windows_sandbox.rs",
  "src-tauri/tests/assistant_host/parity.rs",
];
const sourceBytes = (name, bytes) =>
  /\.(?:rs|mjs|js|ts|py|md|json|toml|svg|txt|typ)$/.test(name)
    ? Buffer.from(bytes.toString("utf8").replace(/\r\n/g, "\n"))
    : bytes;
const SCENARIOS = [
  {
    id: "clip8",
    prompt:
      "直接生成且交付一份恰好8页的中文PPT，向计算机专业本科生介绍CLIP：1问题与动机，2双编码器，3对比学习，4训练流程，5零样本分类，6图文检索，7局限与偏差，8总结。技术事实准确，不编造实验数字、客户或产品能力。使用可编辑原生图形解释机制，不生成付费图片；不要额外加封面或结束页。",
  },
  {
    id: "transformer8",
    prompt:
      "直接生成且交付一份恰好8页的中文PPT，向计算机专业本科生解释Transformer：1问题，2token和位置，3Q/K/V，4注意力，5多头，6前馈与残差，7训练与推理，8局限。第4页必须含可编辑原生公式 Attention(Q,K,V)=softmax(QK^T/sqrt(d_k))V；第7页含原生可编辑柱状图，明确是合成示例，序列长度128/256/512对应示例延迟10/18/34毫秒。图表内嵌可编辑工作簿。不把示例当真实benchmark。不生成付费图片，不额外加页。",
    nativeEquation: true,
    nativeChart: true,
  },
  {
    id: "business-edit8",
    prompt:
      "只修改已引用8页合成经营简报的第3页：把标题改为“第三季度经营复盘”，把正文改为“合成示例：收入120、成本80、利润40；单位为万元，非真实公司数据。”。保持其余7页、主题、页序、备注及共享资源不变，不添加或删除页面。使用原稿修订，交付一份新PPT，原文件不动。不要生成图片。",
    sourcePage: 3,
  },
];

function amount(value) {
  const match = /^(\d+)(?:\.(\d{1,12}))?$/.exec(String(value));
  assert.ok(
    match,
    "USD amount must be a nonnegative decimal with at most 12 places",
  );
  return BigInt(match[1]) * UNIT + BigInt((match[2] ?? "").padEnd(12, "0"));
}
function usd(value) {
  const whole = value / UNIT;
  const decimals = (value % UNIT)
    .toString()
    .padStart(12, "0")
    .replace(/0+$/, "");
  return `${whole}.${decimals.padEnd(2, "0")}`;
}
const occupied = (item) => amount(item.costUsd ?? item.reservedUsd);

export function buildCampaign({
  sourceSha,
  gitSha,
  generationFiles = {},
  apex = "ofox.ai",
}) {
  assert.match(sourceSha, /^[a-f0-9]{64}$/);
  assert.ok(["ofox.ai", "ofox.io"].includes(apex));
  const campaign = {
    version: 1,
    suite: "assistant-1701-parity-v1",
    gitSha,
    selectedApex: apex,
    generationFiles,
    generationHash: digest(generationFiles),
    sourceSha,
    models: [...MODELS],
    parameters: {
      protocol: "openai-completions",
      maxOutputTokens: 16000,
      images: false,
      webSearch: false,
    },
    limits: {
      totalUsd: "10.00",
      taskUsd: "1.00",
      hostAllocationsUsd: { macos: "5.00", win11: "5.00" },
    },
    realModelExecuted: false,
    tasks: [],
    win10Replays: [],
  };
  for (const platform of ["macos", "win11"])
    for (const [index, modelId] of MODELS.entries())
      for (const scenario of SCENARIOS) {
        const task = {
          id: `${platform}-m${index + 1}-${scenario.id}`,
          platform,
          modelId,
          scenario: scenario.id,
          prompt: scenario.prompt,
          expectedSlides: 8,
          nativeEquation: scenario.nativeEquation === true,
          nativeChart: scenario.nativeChart === true,
          sourcePage: scenario.sourcePage ?? null,
          sourceSha: scenario.sourcePage ? sourceSha : null,
        };
        task.inputHash = digest({
          ...task,
          generationHash: campaign.generationHash,
          parameters: campaign.parameters,
        });
        campaign.tasks.push(task);
        if (platform === "win11")
          campaign.win10Replays.push({
            id: `win10-replay-m${index + 1}-${scenario.id}`,
            capturedTaskId: task.id,
            capturedPlatform: "win11",
            inputHash: task.inputHash,
          });
      }
  campaign.campaignHash = digest(campaign);
  return campaign;
}

export function newLedger(campaign) {
  return {
    version: 1,
    campaignHash: campaign.campaignHash,
    stopped: null,
    allocations: { ...campaign.limits.hostAllocationsUsd },
    requests: [],
    tasks: {},
    hostApprovals: [],
  };
}
export function reserve(ledger, task, requestId, upperUsd) {
  assert.ok(!ledger.stopped, "campaign stopped: unknown or exceeded cost");
  assert.ok(
    !ledger.requests.some((item) => item.id === requestId),
    "duplicate request id",
  );
  const upper = amount(upperUsd);
  const sum = (where) =>
    ledger.requests
      .filter(where)
      .reduce((total, item) => total + occupied(item), 0n);
  assert.ok(
    sum((item) => item.taskId === task.id) + upper <= UNIT,
    "task budget exhausted",
  );
  assert.ok(
    sum((item) => item.platform === task.platform) + upper <=
      amount(ledger.allocations[task.platform]),
    "host allocation exhausted",
  );
  assert.ok(sum(() => true) + upper <= 10n * UNIT, "campaign budget exhausted");
  ledger.requests.push({
    id: requestId,
    taskId: task.id,
    platform: task.platform,
    modelId: task.modelId,
    reservedUsd: usd(upper),
    costUsd: null,
    status: "reserved",
  });
}
export function settle(ledger, requestId, costUsd) {
  const item = ledger.requests.find((request) => request.id === requestId);
  assert.ok(
    item && item.status === "reserved",
    "request is not awaiting a cost receipt",
  );
  if (costUsd === null) {
    item.status = "unknown";
    ledger.stopped = "unknown_cost";
    return;
  }
  const cost = amount(costUsd);
  item.status = "known";
  item.costUsd = usd(cost);
  if (cost > amount(item.reservedUsd))
    ledger.stopped = "cost_exceeds_reservation";
}

async function save(file, value) {
  const temporary = `${file}.${randomUUID()}.tmp`;
  const handle = await open(temporary, "wx", 0o600);
  try {
    await handle.writeFile(JSON.stringify(value, null, 2) + "\n");
    await handle.sync();
  } finally {
    await handle.close();
  }
  await rename(temporary, file);
}
export async function withLedgerLock(directory, operation) {
  const file = path.join(directory, "ledger.lock");
  let lock;
  try {
    lock = await open(file, "wx", 0o600);
  } catch (error) {
    if (error.code === "EEXIST")
      throw new Error(
        "campaign ledger locked; never run a second controller or remove an active lock",
      );
    throw error;
  }
  try {
    await lock.writeFile(
      JSON.stringify({ pid: process.pid, host: os.hostname() }),
    );
    return await operation();
  } finally {
    await lock.close();
    await rm(file);
  }
}

async function businessSource() {
  const { default: PptxGenJS } = await import("pptxgenjs");
  const JSZip = createRequire(import.meta.resolve("pptxgenjs"))("jszip");
  const deck = new PptxGenJS();
  deck.layout = "LAYOUT_WIDE";
  deck.author = "Ofox synthetic parity fixture";
  deck.title = "Synthetic business review";
  const titles = [
    "合成经营简报",
    "业务概览",
    "待修改页",
    "收入结构",
    "成本结构",
    "交付效率",
    "风险与限制",
    "后续计划",
  ];
  for (const [index, title] of titles.entries()) {
    const slide = deck.addSlide();
    slide.background = { color: "F5F7FA" };
    slide.addText(title, {
      x: 0.8,
      y: 0.5,
      w: 11.6,
      h: 0.7,
      fontFace: "Arial",
      fontSize: 28,
      color: "17304D",
    });
    slide.addText(`合成示例第${index + 1}页；非真实公司数据。`, {
      x: 0.8,
      y: 2,
      w: 11.6,
      h: 1,
      fontFace: "Arial",
      fontSize: 22,
    });
    slide.addNotes(`Preserve synthetic note ${index + 1}`);
  }
  const source = await JSZip.loadAsync(
    await deck.write({ outputType: "nodebuffer" }),
  );
  const result = new JSZip();
  for (const name of Object.keys(source.files).sort())
    if (!source.files[name].dir) {
      let bytes = await source.files[name].async("nodebuffer");
      if (name === "docProps/core.xml")
        bytes = Buffer.from(
          bytes
            .toString()
            .replace(
              /\d{4}-\d\d-\d\dT\d\d:\d\d:\d\dZ/g,
              "2026-11-16T00:00:00Z",
            ),
        );
      result.file(name, bytes, {
        date: new Date("2026-11-16T00:00:00Z"),
        createFolders: false,
      });
    }
  return result.generateAsync({ type: "nodebuffer", compression: "DEFLATE" });
}
async function snapshot() {
  const files = {};
  async function walk(relative) {
    for (const entry of await readdir(path.join(root, relative), {
      withFileTypes: true,
    })) {
      const name = path.posix.join(relative, entry.name);
      if (entry.isDirectory()) await walk(name);
      else if (entry.isFile()) {
        const bytes = await readFile(path.join(root, name));
        // Git text checkout conventions may differ between the native hosts.
        files[name] = hash(sourceBytes(name, bytes));
      }
    }
  }
  for (const directory of [
    "assistant-runtime/src",
    "assistant-runtime/skills/ofox-office-pptx",
    "src-tauri/src/services/assistant",
  ])
    await walk(directory);
  for (const name of [
    "scripts/assistant-parity-eval.mjs",
    "src-tauri/tests/assistant_host/parity.rs",
  ])
    files[name] = hash(
      sourceBytes(name, await readFile(path.join(root, name))),
    );
  return Object.fromEntries(Object.entries(files).sort());
}
export async function prepare(directory, { apex = "ofox.ai" } = {}) {
  await mkdir(directory, { recursive: true });
  const marker = await open(
    path.join(directory, "campaign.json"),
    "wx",
    0o600,
  ).catch((error) => {
    if (error.code === "EEXIST")
      throw new Error("campaign already exists; never reset a paid ledger");
    throw error;
  });
  await marker.close();
  const source = await businessSource();
  const campaign = buildCampaign({
    sourceSha: hash(source),
    gitSha: execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: root,
      encoding: "utf8",
    }).trim(),
    generationFiles: await snapshot(),
    apex,
  });
  const sourceFile = await open(
    path.join(directory, "business-source.pptx"),
    "wx",
    0o600,
  );
  try {
    await sourceFile.writeFile(source);
  } finally {
    await sourceFile.close();
  }
  await save(path.join(directory, "campaign.json"), campaign);
  await save(path.join(directory, "ledger.json"), newLedger(campaign));
  await save(path.join(directory, "report.json"), summary(campaign, []));
  return campaign;
}

export function validateReceipt(
  campaign,
  task,
  receipt,
  { manual = true } = {},
) {
  assert.ok(
    receipt.taskId === task.id &&
      receipt.modelId === task.modelId &&
      receipt.inputHash === task.inputHash &&
      receipt.gitSha === campaign.gitSha &&
      receipt.generationHash === campaign.generationHash,
    "receipt identity does not match the frozen task",
  );
  assert.ok(
    receipt.compiledSourceSha === campaign.gitSha &&
      receipt.compiledSourceClean === true,
    "compiled clean source identity is missing",
  );
  const actual = receipt.nativeHost;
  const expectedPlatform = receipt.mode === "replay" ? "win10" : task.platform;
  assert.ok(
    actual?.platform === expectedPlatform &&
      (expectedPlatform === "macos" ||
        (classifyWindows(actual?.osRelease, actual?.productType) ===
          expectedPlatform &&
          actual?.kernelProductType === 1)),
    "native client platform proof does not match the task",
  );
  assert.ok(receipt.status === "completed", "task did not complete");
  assert.ok(
    receipt.artifact &&
      /^[a-f0-9]{64}$/.test(receipt.artifact.sha256 ?? "") &&
      receipt.artifact.path &&
      receipt.generatedAttachments === 1,
    "missing actual artifact proof",
  );
  assert.ok(
    receipt.actualSlides === 8 &&
      receipt.verifiedArtifact?.slides === 8 &&
      receipt.verifiedArtifact.sha256 === receipt.artifact.sha256,
    "actual artifact has not been independently verified",
  );
  assert.ok(
    receipt.route === "python" &&
      receipt.engineExecuted === true &&
      receipt.isolationVerified === true,
    "isolated application engine was not verified",
  );
  if (task.platform === "win11")
    assert.ok(
      receipt.isolationReceipts?.some(
        (proof) =>
          proof.policy === "lpac-v1" &&
          proof.trustedProbe === false &&
          proof.tokenVerified === true &&
          proof.childProcessDenied === true &&
          proof.lowImageDenied === true &&
          proof.jobStopped === true &&
          proof.cleanupComplete === true &&
          proof.taskRemoved === true &&
          proof.profileRemoved === true &&
          proof.cancelled === false &&
          proof.status === 0 &&
          proof.executable?.replaceAll("\\", "/").startsWith("node/") &&
          Number.isSafeInteger(proof.sequence) &&
          proof.sequence > 0 &&
          proof.runId &&
          proof.taskId &&
          /^[a-f0-9]{64}$/.test(proof.runtimeSha256 ?? ""),
      ),
      "fresh native LPAC execution and cleanup proof missing",
    );
  assert.ok(
    /^[a-f0-9]{64}$/.test(receipt.captureSha256 ?? "") &&
      typeof receipt.frozenPlan?.planId === "string" &&
      receipt.frozenPlan?.revision === 1 &&
      receipt.checks?.validation === true &&
      receipt.checks?.renderedPages === 8,
    "incomplete capture, plan, validation or page rendering",
  );
  if (task.nativeEquation)
    assert.ok(
      receipt.verifiedArtifact.nativeEquation,
      "native equation missing",
    );
  if (task.nativeChart)
    assert.ok(
      receipt.verifiedArtifact.nativeChart &&
        receipt.verifiedArtifact.embeddedWorkbook,
      "editable chart workbook missing",
    );
  if (task.sourcePage)
    assert.ok(
      receipt.sourceSha === task.sourceSha &&
        receipt.verifiedArtifact.onlyPage3Changed === true &&
        receipt.frozenPlan.source?.version === task.sourceSha &&
        canonical(receipt.frozenPlan.source?.pageNumbers) === "[3]" &&
        receipt.parentRevision?.parentVersion === task.sourceSha &&
        canonical(receipt.parentRevision?.pageNumbers) === "[3]",
      "source invariants failed",
    );
  if (manual) {
    const review = (field) =>
      receipt[field]?.status === "passed" &&
      receipt[field]?.artifactSha256 === receipt.artifact.sha256 &&
      receipt[field]?.evidence?.path &&
      /^[a-f0-9]{64}$/.test(receipt[field]?.evidence?.sha256 ?? "");
    assert.ok(
      review("visualReview") &&
        canonical(receipt.visualReview.reviewedPages) ===
          canonical([1, 2, 3, 4, 5, 6, 7, 8]) &&
        review("targetApplicationReview") &&
        receipt.targetApplicationReview.application &&
        receipt.targetApplicationReview.version &&
        receipt.targetApplicationReview.savedAndReopened === true,
      "visual and target-application acceptance remain pending",
    );
  }
  return true;
}
function budgetStatus(campaign, ledger) {
  if (!ledger) return { status: "pending", reason: "shared ledger missing" };
  try {
    assert.equal(
      ledger.campaignHash,
      campaign.campaignHash,
      "ledger belongs to another campaign",
    );
    assert.equal(
      canonical(ledger.allocations),
      canonical(campaign.limits.hostAllocationsUsd),
      "host allocations changed",
    );
    const byTask = new Map(),
      byHost = new Map(),
      ids = new Set();
    let total = 0n,
      unresolved = false;
    for (const item of ledger.requests) {
      assert.ok(!ids.has(item.id), "duplicate request receipt");
      ids.add(item.id);
      const task = campaign.tasks.find((task) => task.id === item.taskId);
      assert.ok(
        task &&
          task.modelId === item.modelId &&
          task.platform === item.platform,
        "request identity differs from the matrix",
      );
      assert.ok(
        ["known", "reserved", "unknown"].includes(item.status),
        "invalid request outcome",
      );
      if (item.status === "known")
        assert.ok(
          item.costUsd !== null &&
            amount(item.costUsd) <= amount(item.reservedUsd),
          "cost exceeded its reservation",
        );
      else unresolved = true;
      const cost = occupied(item);
      total += cost;
      byTask.set(task.id, (byTask.get(task.id) ?? 0n) + cost);
      byHost.set(task.platform, (byHost.get(task.platform) ?? 0n) + cost);
    }
    assert.ok(
      total <= 10n * UNIT &&
        [...byTask.values()].every((cost) => cost <= UNIT) &&
        [...byHost.values()].every((cost) => cost <= 5n * UNIT),
      "campaign, task or host budget exceeded",
    );
    const complete = campaign.tasks.every(
      (task) =>
        ledger.tasks[task.id]?.status === "completed" &&
        ledger.tasks[task.id]?.inputHash === task.inputHash &&
        byTask.has(task.id),
    );
    if (ledger.stopped && ledger.stopped !== "unknown_cost")
      return {
        status: "failed",
        reason: ledger.stopped,
        occupiedUsd: usd(total),
      };
    return {
      status: unresolved || ledger.stopped || !complete ? "pending" : "passed",
      occupiedUsd: usd(total),
      reason:
        unresolved || ledger.stopped
          ? "unknown or unfinished request cost; reservation retained"
          : !complete
            ? "matrix tasks remain unexecuted"
            : "captured token-cost estimates known; not invoice reconciliation",
    };
  } catch (error) {
    return { status: "failed", reason: error.message };
  }
}
export function summary(campaign, receipts, replays = [], ledger = null) {
  const records = campaign.tasks.map((task) => {
    const receipt = receipts.find((item) => item.taskId === task.id);
    if (!receipt)
      return { taskId: task.id, status: "pending", reason: "not executed" };
    try {
      validateReceipt(campaign, task, receipt, { manual: false });
    } catch (error) {
      return { taskId: task.id, status: "failed", reason: error.message };
    }
    try {
      validateReceipt(campaign, task, receipt);
      return { taskId: task.id, status: "passed" };
    } catch (error) {
      return { taskId: task.id, status: "pending", reason: error.message };
    }
  });
  const replayPassed = campaign.win10Replays.every((expected) =>
    replays.some(
      (receipt) =>
        receipt.replayId === expected.id &&
        receipt.accepted === true &&
        receipt.manualAccepted === true,
    ),
  );
  const budget = budgetStatus(campaign, ledger);
  return {
    suite: campaign.suite,
    campaignHash: campaign.campaignHash,
    status:
      records.some((record) => record.status === "failed") ||
      budget.status === "failed"
        ? "failed"
        : records.every((record) => record.status === "passed") &&
            replayPassed &&
            budget.status === "passed"
          ? "passed"
          : "pending",
    tasks: records,
    budget,
    win10Replay: {
      status: replayPassed ? "passed" : "pending",
      expected: 6,
      reason:
        "requires native Win10 application LPAC replay of all six Win11 captures",
    },
    note: "All twelve tasks and six native Win10 replays must pass; no tolerated failures. Model/engine receipts alone do not certify Office editing or visual quality.",
  };
}

function readKey() {
  if (process.env.OFOX_API_KEY?.trim()) return process.env.OFOX_API_KEY.trim();
  if (process.platform !== "darwin") return "";
  try {
    return execFileSync(
      "/usr/bin/security",
      [
        "find-generic-password",
        "-s",
        "ai.ofox.desktop.apikey",
        "-a",
        "assistant-1701-eval",
        "-w",
      ],
      {
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
        timeout: 30000,
        maxBuffer: 8192,
      },
    ).trim();
  } catch {
    return "";
  }
}
export function classifyWindows(release, productType) {
  if (productType !== "WinNT" || !/^10\.0\.\d+$/.test(release))
    return "unsupported";
  const build = Number(release.split(".")[2]);
  return build >= 22000 ? "win11" : build >= 17763 ? "win10" : "unsupported";
}
function nativePlatform() {
  if (process.platform === "darwin") return "macos";
  if (process.platform !== "win32" || !process.env.SystemRoot)
    return "unsupported";
  try {
    const result = execFileSync(
      path.join(process.env.SystemRoot, "System32/reg.exe"),
      [
        "query",
        "HKLM\\SYSTEM\\CurrentControlSet\\Control\\ProductOptions",
        "/v",
        "ProductType",
      ],
      { encoding: "utf8", timeout: 10000, stdio: ["ignore", "pipe", "ignore"] },
    );
    const productType = /^\s*ProductType\s+REG_SZ\s+(\S+)\s*$/m.exec(
      result,
    )?.[1];
    return classifyWindows(os.release(), productType);
  } catch {
    return "unsupported";
  }
}
function price(row, field) {
  const value = row.pricing?.[field];
  assert.ok(
    typeof value === "string" && /^\d+(?:\.\d{1,12})?$/.test(value),
    "catalog token price is unknown or unsupported",
  );
  return amount(value);
}
function usageCost(text, row) {
  let usage;
  for (const line of text.split(/\r?\n/))
    if (line.startsWith("data:")) {
      try {
        const value = JSON.parse(line.slice(5));
        if (value.usage) usage = value.usage;
      } catch {
        /* SSE DONE is not a receipt. */
      }
    }
  assert.ok(
    Number.isSafeInteger(usage?.prompt_tokens) &&
      usage.prompt_tokens >= 0 &&
      Number.isSafeInteger(usage?.completion_tokens) &&
      usage.completion_tokens >= 0,
    "missing usage",
  );
  assert.ok(
    usage.prompt_tokens <= row.context_length &&
      usage.completion_tokens <= 16000,
    "usage exceeds admitted bounds",
  );
  return usd(
    BigInt(usage.prompt_tokens) * price(row, "prompt") +
      BigInt(usage.completion_tokens) * price(row, "completion"),
  );
}
async function limitedBody(request, maximum = 8 * 1024 * 1024) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    assert.ok(size <= maximum, "request size exceeded");
    chunks.push(chunk);
  }
  return Buffer.concat(chunks);
}

export async function forwarder({
  ledger,
  directory,
  key,
  apex,
  catalog,
  fetchImpl = fetch,
}) {
  let task = null,
    busy = false,
    activeAbort = null,
    drained = Promise.resolve();
  const token = `parity-proxy-${randomUUID()}-${randomUUID()}`;
  const persist = () => save(path.join(directory, "ledger.json"), ledger);
  const server = createServer(async (request, response) => {
    let requestId,
      done,
      admitted = false;
    const fail = (message) => {
      if (!response.headersSent)
        response.writeHead(403, { "content-type": "application/json" });
      response.end(
        JSON.stringify({ error: { message: redact(message, [key, token]) } }),
      );
    };
    try {
      assert.equal(
        request.headers.authorization,
        `Bearer ${token}`,
        "unknown controller token",
      );
      if (request.method === "GET" && request.url === "/v1/models") {
        response.writeHead(200, { "content-type": "application/json" });
        response.end(JSON.stringify(catalog));
        return;
      }
      assert.ok(
        task &&
          !busy &&
          request.method === "POST" &&
          request.url === "/v1/chat/completions",
        "only one admitted text request at a time",
      );
      busy = true;
      admitted = true;
      drained = new Promise((resolve) => {
        done = resolve;
      });
      const body = JSON.parse((await limitedBody(request)).toString());
      assert.equal(
        body.model,
        task.modelId,
        "model differs from exact frozen ID",
      );
      assert.equal(body.stream, true, "streamed usage receipt required");
      const cap = body.max_completion_tokens ?? body.max_tokens;
      assert.ok(
        Number.isSafeInteger(cap) && cap > 0 && cap <= 16000,
        "output cap missing or exceeded",
      );
      body.stream_options = {
        ...(body.stream_options ?? {}),
        include_usage: true,
      };
      const row = catalog.data.find((item) => item.id === task.modelId);
      assert.ok(
        Number.isSafeInteger(row?.context_length) && row.context_length > 0,
        "catalog context bound missing",
      );
      requestId = randomUUID();
      reserve(
        ledger,
        task,
        requestId,
        usd(
          BigInt(row.context_length) * price(row, "prompt") +
            BigInt(cap) * price(row, "completion"),
        ),
      );
      ledger.requests.at(-1).priceEvidence = {
        catalogSha256: digest(catalog),
        modelId: row.id,
        pricing: row.pricing,
        contextLength: row.context_length,
        maxOutputTokens: cap,
      };
      await persist(); // Durable reservation precedes the sole paid operation.
      activeAbort = new AbortController();
      const upstream = await fetchImpl(
        `https://api.${apex}/v1/chat/completions`,
        {
          method: "POST",
          redirect: "error",
          headers: {
            Authorization: `Bearer ${key}`,
            "content-type": "application/json",
          },
          body: JSON.stringify(body),
          signal: AbortSignal.any([
            activeAbort.signal,
            AbortSignal.timeout(180000),
          ]),
        },
      );
      const chunks = [];
      let size = 0;
      for await (const chunk of upstream.body) {
        size += chunk.length;
        assert.ok(size <= 16 * 1024 * 1024, "response exceeds receipt bound");
        chunks.push(Buffer.from(chunk));
      }
      const text = Buffer.concat(chunks).toString();
      assert.ok(upstream.ok, "submitted request failed; billing unknown");
      settle(ledger, requestId, usageCost(text, row));
      await persist();
      // Never forward the real credential, even if a remote diagnostic echoes
      // it across stream chunks. This bounded evaluation proxy buffers SSE.
      response.writeHead(upstream.status, {
        "content-type":
          upstream.headers.get("content-type") ?? "text/event-stream",
      });
      response.end(text.split(key).join("[REDACTED]"));
    } catch (error) {
      if (
        requestId &&
        ledger.requests.find((item) => item.id === requestId)?.status ===
          "reserved"
      ) {
        settle(ledger, requestId, null);
        await persist();
      }
      fail(error.message);
    } finally {
      if (admitted) {
        busy = false;
        activeAbort = null;
        done();
      }
    }
  });
  await new Promise((resolve) => server.listen(0, "localhost", resolve));
  return {
    base: `http://localhost:${server.address().port}`,
    token,
    select: (selected) => {
      assert.ok(!busy);
      task = selected;
    },
    close: async () => {
      activeAbort?.abort();
      server.closeAllConnections();
      await drained;
      await new Promise((resolve) => server.close(resolve));
    },
  };
}

async function runDriver(
  driver,
  task,
  directory,
  campaign,
  proxy,
  mode = "live",
) {
  const out = path.join(
    directory,
    mode === "live" ? task.id : `win10-replay-${task.id.slice(6)}`,
  );
  await mkdir(out); // One attempt; never overwrite or silently rerun a task.
  await save(path.join(out, "input.json"), {
    ...task,
    gitSha: campaign.gitSha,
    generationHash: campaign.generationHash,
    parameters: campaign.parameters,
    selectedApex: campaign.selectedApex,
    nativeHost: {
      platform: nativePlatform(),
      osRelease: os.release(),
      architecture: process.arch,
    },
    driverSha256: hash(await readFile(driver)),
    compiledSources: Object.fromEntries(
      COMPILED_SOURCES.map((name) => [name, campaign.generationFiles[name]]),
    ),
  });
  const home = path.join(out, "home");
  await mkdir(home);
  const env = Object.fromEntries(
    ["PATH", "SystemRoot", "WINDIR", "COMSPEC"]
      .filter((name) => process.env[name])
      .map((name) => [name, process.env[name]]),
  );
  Object.assign(env, {
    HOME: home,
    USERPROFILE: home,
    TMPDIR: home,
    TEMP: home,
    TMP: home,
    OFOX_PARITY_ACK: "USD10_TOTAL_USD1_TASK_NO_IMAGES",
    OFOX_PARITY_CASE: path.join(out, "input.json"),
    OFOX_PARITY_OUT: out,
    OFOX_PARITY_RUNTIME: path.join(root, "assistant-runtime/resources"),
    OFOX_PARITY_CATALOG: path.join(directory, `catalog-${task.platform}.json`),
    OFOX_PARITY_SOURCE: path.join(directory, "business-source.pptx"),
    OFOX_PARITY_PROXY_BASE: proxy.base,
    OFOX_PARITY_PROXY_TOKEN: proxy.token,
    OFOX_PARITY_MODE: mode,
  });
  if (mode === "replay")
    env.OFOX_PARITY_CAPTURE = path.join(directory, task.id, "capture.json");
  const code = await new Promise((resolve) => {
    const child = spawn(
      driver,
      [
        "parity_eval::one_guarded_task",
        "--ignored",
        "--exact",
        "--nocapture",
        "--test-threads=1",
      ],
      { cwd: root, env, stdio: ["ignore", "pipe", "pipe"] },
    );
    const timer = setTimeout(() => child.kill(), 20 * 60 * 1000);
    let total = 0;
    for (const stream of [child.stdout, child.stderr])
      stream.on("data", (chunk) => {
        total += chunk.length;
        if (total <= 1024 * 1024)
          process.stderr.write(redact(chunk.toString(), [proxy.token]));
      });
    child.on("error", () => {
      clearTimeout(timer);
      resolve(null);
    });
    child.on("close", (code) => {
      clearTimeout(timer);
      resolve(code);
    });
  });
  let receipt;
  try {
    receipt = JSON.parse(
      await readFile(path.join(out, "outcome.json"), "utf8"),
    );
  } catch {
    throw new Error(`task ${task.id}: no Host receipt (driver exit ${code})`);
  }
  assert.equal(
    code,
    0,
    `task ${task.id}: driver failed; inspect its receipt, never rerun automatically`,
  );
  return { out, receipt };
}

async function xmlTags(text) {
  const { SaxesParser } = createRequire(
    new URL("../assistant-runtime/package.json", import.meta.url),
  )("saxes");
  const parser = new SaxesParser({ xmlns: true });
  const tags = [],
    fingerprint = [],
    parents = [];
  parser.on("doctype", () => {
    throw new Error("DTD is not accepted");
  });
  parser.on("opentag", (tag) => {
    const attrs = Object.values(tag.attributes)
      .map((item) => [item.uri, item.local, item.value])
      .sort((a, b) => canonical(a).localeCompare(canonical(b)));
    tags.push({
      uri: tag.uri,
      local: tag.local,
      attrs: Object.values(tag.attributes),
    });
    fingerprint.push(["start", tag.uri, tag.local, attrs]);
    parents.push(tag.local);
  });
  parser.on("closetag", () => {
    parents.pop();
    fingerprint.push(["end"]);
  });
  parser.on("text", (text) => {
    if (text.trim() || parents.at(-1) === "t") fingerprint.push(["text", text]);
  });
  parser.on("cdata", (text) => fingerprint.push(["text", text]));
  parser.write(text).close();
  return { tags, fingerprint: canonical(fingerprint) };
}
export async function boundedZip(bytes) {
  const yauzl = createRequire(
    new URL("../assistant-runtime/package.json", import.meta.url),
  )("yauzl");
  const deadline = Date.now() + 60000;
  let expanded = 0,
    decoded = 0;
  return new Promise((resolve, reject) => {
    yauzl.fromBuffer(
      bytes,
      {
        lazyEntries: true,
        autoClose: false,
        validateEntrySizes: true,
        strictFileNames: true,
      },
      (error, zip) => {
        if (error) {
          reject(error);
          return;
        }
        const files = Object.create(null);
        const fail = (error) => {
          zip.close();
          reject(error);
        };
        zip.on("error", fail);
        if (zip.entryCount > 10000) {
          fail(new Error("archive inspection bound: too many entries"));
          return;
        }
        const read = (entry, type) =>
          new Promise((resolve, reject) => {
            const maximum = /\.(?:xml|rels)$/.test(entry.fileName)
              ? 4 * 1024 * 1024
              : 32 * 1024 * 1024;
            zip.openReadStream(entry, (error, stream) => {
              if (error) {
                reject(error);
                return;
              }
              let size = 0,
                checksum = 0;
              const chunks = [];
              const timer = setTimeout(
                () => stream.destroy(new Error("archive inspection deadline")),
                Math.max(1, deadline - Date.now()),
              );
              stream.on("data", (chunk) => {
                size += chunk.length;
                decoded += chunk.length;
                if (
                  size > maximum ||
                  size > entry.uncompressedSize ||
                  decoded > 128 * 1024 * 1024 ||
                  Date.now() > deadline
                ) {
                  stream.destroy(
                    new Error("archive inspection bound: decoded bytes"),
                  );
                  return;
                }
                checksum = crc32(chunk, checksum);
                chunks.push(chunk);
              });
              stream.once("error", (error) => {
                clearTimeout(timer);
                reject(error);
              });
              stream.once("end", () => {
                clearTimeout(timer);
                if (
                  size !== entry.uncompressedSize ||
                  checksum >>> 0 !== entry.crc32 >>> 0
                ) {
                  reject(new Error("archive CRC/size mismatch"));
                  return;
                }
                const result = Buffer.concat(chunks);
                resolve(type === "string" ? result.toString("utf8") : result);
              });
            });
          });
        zip.on("entry", (entry) => {
          try {
            const name = entry.fileName,
              maximum = /\.(?:xml|rels)$/.test(name)
                ? 4 * 1024 * 1024
                : 32 * 1024 * 1024;
            assert.ok(
              !name.includes("\\") &&
                !name.includes(":") &&
                !name.startsWith("/") &&
                !name.split("/").includes("..") &&
                !files[name],
              "invalid or duplicate archive part",
            );
            assert.ok(
              Number.isSafeInteger(entry.uncompressedSize) &&
                entry.uncompressedSize <= maximum &&
                entry.uncompressedSize >= 0 &&
                Date.now() <= deadline,
              "archive inspection bound: declared entry size",
            );
            expanded += entry.uncompressedSize;
            assert.ok(
              expanded <= 100 * 1024 * 1024,
              "archive inspection bound: declared expansion",
            );
            files[name] = {
              dir: name.endsWith("/"),
              _data: { uncompressedSize: entry.uncompressedSize },
              async: (type) => read(entry, type),
            };
            zip.readEntry();
          } catch (error) {
            fail(error);
          }
        });
        zip.once("end", () =>
          resolve({
            files,
            file: (name) => files[name],
            close: () => zip.close(),
          }),
        );
        zip.readEntry();
      },
    );
  });
}
export function assertCapturedPlan(input, frozenPlan) {
  assert.equal(
    canonical(input?.requirements),
    canonical(frozenPlan?.requirements),
    "captured requirements differ from frozen Host plan",
  );
  assert.equal(
    canonical(normalizePresentationPlanInput(input)?.slides),
    canonical(frozenPlan?.slides),
    "captured pages differ from frozen Host plan",
  );
}

async function inspectArtifact(directory, task, receipt, sourcePath) {
  const file = path.resolve(directory, receipt.artifact.path);
  assert.ok(
    file.startsWith(directory + path.sep) && (await lstat(file)).isFile(),
    "artifact escaped its task directory",
  );
  const bytes = await readFile(file);
  assert.ok(
    bytes.length > 0 && bytes.length <= 25 * 1024 * 1024,
    "artifact exceeds the application file bound",
  );
  const zip = await boundedZip(bytes);
  let expanded = 0;
  assert.ok(Object.keys(zip.files).length <= 10000, "too many archive parts");
  for (const item of Object.values(zip.files))
    if (!item.dir) {
      const size = item._data?.uncompressedSize;
      assert.ok(
        Number.isSafeInteger(size) && size >= 0 && size <= 32 * 1024 * 1024,
        "archive part exceeds inspection bound",
      );
      expanded += size;
    }
  assert.ok(
    expanded <= 100 * 1024 * 1024,
    "archive expansion exceeds inspection bound",
  );
  const relNS = "http://schemas.openxmlformats.org/package/2006/relationships";
  const pNS = "http://schemas.openxmlformats.org/presentationml/2006/main";
  const rNS =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
  const attribute = (tag, local, uri = "") =>
    tag.attrs.find((item) => item.local === local && item.uri === uri)?.value;
  const resolvePart = (base, target) => {
    assert.ok(
      target &&
        !target.includes(":") &&
        !target.includes("\\") &&
        !target.startsWith("/"),
      "invalid OPC part target",
    );
    const name = path.posix.normalize(
      path.posix.join(path.posix.dirname(base), target),
    );
    assert.ok(
      !name.startsWith("../") && !name.startsWith("/"),
      "OPC part escapes archive",
    );
    return name;
  };
  const roots = (
    await xmlTags(await zip.file("_rels/.rels")?.async("string"))
  ).tags.filter(
    (tag) =>
      tag.uri === relNS &&
      tag.local === "Relationship" &&
      attribute(tag, "Type") === `${rNS}/officeDocument`,
  );
  assert.equal(
    roots.length,
    1,
    "one canonical office-document relationship required",
  );
  const main = resolvePart("root.xml", attribute(roots[0], "Target"));
  const document = await zip.file(main)?.async("string");
  assert.ok(document, "main presentation part missing");
  const slideIds = (await xmlTags(document)).tags.filter(
    (tag) => tag.uri === pNS && tag.local === "sldId",
  );
  const mainRels = path.posix.join(
    path.posix.dirname(main),
    "_rels",
    path.posix.basename(main) + ".rels",
  );
  const relationships = (
    await xmlTags(await zip.file(mainRels)?.async("string"))
  ).tags.filter((tag) => tag.uri === relNS && tag.local === "Relationship");
  const slidePaths = slideIds.map((tag) => {
    const relation = relationships.find(
      (entry) => attribute(entry, "Id") === attribute(tag, "id", rNS),
    );
    assert.ok(
      relation &&
        attribute(relation, "Type") === `${rNS}/slide` &&
        attribute(relation, "TargetMode") !== "External",
      "invalid slide relationship",
    );
    const name = resolvePart(main, attribute(relation, "Target"));
    assert.ok(zip.file(name), "slide part missing");
    return name;
  });
  assert.equal(
    new Set(slidePaths).size,
    slidePaths.length,
    "duplicate canonical slide target",
  );
  const slides = slidePaths.length;
  let nativeEquation = false,
    nativeChart = false,
    embeddedWorkbook = false;
  const mNS = "http://schemas.openxmlformats.org/officeDocument/2006/math";
  const cNS = "http://schemas.openxmlformats.org/drawingml/2006/chart";
  for (const slide of slidePaths) {
    const tags = (await xmlTags(await zip.file(slide).async("string"))).tags;
    nativeEquation ||= tags.some(
      (tag) => tag.uri === mNS && ["oMath", "oMathPara"].includes(tag.local),
    );
    const refs = tags.filter((tag) => tag.uri === cNS && tag.local === "chart");
    if (!refs.length) continue;
    const rels = path.posix.join(
      path.posix.dirname(slide),
      "_rels",
      path.posix.basename(slide) + ".rels",
    );
    const slideRels = (await xmlTags(await zip.file(rels)?.async("string")))
      .tags;
    for (const chart of refs) {
      const relation = slideRels.find(
        (tag) =>
          tag.uri === relNS &&
          attribute(tag, "Id") === attribute(chart, "id", rNS) &&
          attribute(tag, "Type") === `${rNS}/chart` &&
          attribute(tag, "TargetMode") !== "External",
      );
      assert.ok(relation, "chart relationship missing");
      const chartPath = resolvePart(slide, attribute(relation, "Target"));
      const chartTags = (
        await xmlTags(await zip.file(chartPath)?.async("string"))
      ).tags;
      nativeChart ||= chartTags.some(
        (tag) => tag.uri === cNS && tag.local === "chartSpace",
      );
      const externalData = chartTags.find(
        (tag) => tag.uri === cNS && tag.local === "externalData",
      );
      if (!externalData) continue;
      const chartRels = path.posix.join(
        path.posix.dirname(chartPath),
        "_rels",
        path.posix.basename(chartPath) + ".rels",
      );
      const dataRelation = (
        await xmlTags(await zip.file(chartRels)?.async("string"))
      ).tags.find(
        (tag) =>
          tag.uri === relNS &&
          attribute(tag, "Id") === attribute(externalData, "id", rNS) &&
          attribute(tag, "Type") === `${rNS}/package` &&
          attribute(tag, "TargetMode") !== "External",
      );
      assert.ok(dataRelation, "embedded chart data relationship missing");
      const workbook = zip.file(
        resolvePart(chartPath, attribute(dataRelation, "Target")),
      );
      assert.ok(workbook, "embedded chart workbook missing");
      const book = await boundedZip(await workbook.async("nodebuffer"));
      const workbookXml = await book.file("xl/workbook.xml")?.async("string");
      embeddedWorkbook ||= Boolean(
        workbookXml &&
        (await xmlTags(workbookXml)).tags.some(
          (tag) =>
            tag.local === "workbook" &&
            tag.uri ===
              "http://schemas.openxmlformats.org/spreadsheetml/2006/main",
        ),
      );
      book.close();
    }
  }
  let onlyPage3Changed = false;
  if (task.sourcePage) {
    const source = await readFile(sourcePath);
    assert.equal(hash(source), task.sourceSha, "source fixture changed");
    const original = await boundedZip(source);
    const changed = [];
    for (const name of new Set([
      ...Object.keys(original.files),
      ...Object.keys(zip.files),
    ])) {
      if (original.files[name]?.dir || zip.files[name]?.dir) continue;
      const left = await original.file(name)?.async("nodebuffer"),
        right = await zip.file(name)?.async("nodebuffer");
      const same =
        left &&
        right &&
        (left.equals(right) ||
          (/\.(?:xml|rels)$/.test(name) &&
            (await xmlTags(left.toString())).fingerprint ===
              (await xmlTags(right.toString())).fingerprint));
      if (!same) changed.push(name);
    }
    onlyPage3Changed =
      changed.length === 1 && changed[0] === "ppt/slides/slide3.xml";
    original.close();
  }
  const capture = await readFile(path.join(directory, "capture.json"));
  assert.equal(
    hash(capture),
    receipt.captureSha256,
    "captured engine input changed",
  );
  zip.close();
  const input = JSON.parse(capture.toString());
  assert.ok(
    input.taskId === task.id &&
      input.modelId === task.modelId &&
      input.inputHash === task.inputHash &&
      input.generationHash === receipt.generationHash,
    "captured input identity mismatch",
  );
  assert.ok(
    input.creates?.length > 0 &&
      input.creates.every(
        (call) =>
          call.format === "pptx" &&
          call.content?.kind === "python" &&
          typeof call.content.code === "string",
      ),
    "capture must contain every exact Python document input",
  );
  assertCapturedPlan(input.freeze, receipt.frozenPlan);
  return {
    sha256: hash(bytes),
    slides,
    nativeEquation,
    nativeChart,
    embeddedWorkbook,
    onlyPage3Changed,
  };
}

async function reviewEvidence(folder, receipt) {
  for (const field of ["visualReview", "targetApplicationReview"]) {
    const review = receipt[field];
    if (review?.status !== "passed") continue;
    const evidence = path.resolve(folder, review.evidence?.path ?? "");
    assert.ok(
      evidence.startsWith(folder + path.sep) &&
        (await lstat(evidence)).isFile(),
      "review evidence escaped its task",
    );
    assert.equal(
      hash(await readFile(evidence)),
      review.evidence.sha256,
      "review evidence changed",
    );
  }
}
async function report(directory, campaign) {
  const receipts = [],
    replays = [];
  for (const task of campaign.tasks) {
    try {
      const folder = path.join(directory, task.id);
      const receipt = JSON.parse(
        await readFile(path.join(folder, "verified-outcome.json"), "utf8"),
      );
      try {
        receipt.verifiedArtifact = await inspectArtifact(
          folder,
          task,
          receipt,
          path.join(directory, "business-source.pptx"),
        );
        await reviewEvidence(folder, receipt);
      } catch (error) {
        receipt.status = "error";
        receipt.verificationError = error.message;
      }
      receipts.push(receipt);
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  for (const replay of campaign.win10Replays) {
    try {
      const folder = path.join(directory, replay.id);
      const saved = JSON.parse(
        await readFile(path.join(folder, "replay-receipt.json"), "utf8"),
      );
      const task = campaign.tasks.find(
        (item) => item.id === replay.capturedTaskId,
      );
      const source = receipts.find((item) => item.taskId === task.id);
      let accepted = false,
        manualAccepted = false;
      try {
        saved.outcome.verifiedArtifact = await inspectArtifact(
          folder,
          task,
          saved.outcome,
          path.join(directory, "business-source.pptx"),
        );
        validateReceipt(campaign, task, saved.outcome, { manual: false });
        accepted =
          saved.replayId === replay.id &&
          saved.capturedTaskId === task.id &&
          saved.outcome.mode === "replay" &&
          saved.capturedInputSha256 === source?.captureSha256 &&
          saved.outcome.replayedCaptureSha256 === source?.captureSha256;
        await reviewEvidence(folder, saved.outcome);
        validateReceipt(campaign, task, saved.outcome);
        manualAccepted = true;
      } catch {
        /* A missing, changed or incomplete replay cannot pass. */
      }
      replays.push({ replayId: replay.id, accepted, manualAccepted });
    } catch (error) {
      if (error.code !== "ENOENT") throw error;
    }
  }
  let ledger;
  try {
    ledger = JSON.parse(
      await readFile(path.join(directory, "ledger.json"), "utf8"),
    );
  } catch (error) {
    if (error.code !== "ENOENT") throw error;
  }
  await save(
    path.join(directory, "report.json"),
    summary(campaign, receipts, replays, ledger),
  );
}

async function verifyRuntimeSource(campaign) {
  for (const [name, expected] of Object.entries(campaign.generationFiles)) {
    if (
      !name.startsWith("assistant-runtime/src/") &&
      !name.startsWith("assistant-runtime/skills/ofox-office-pptx/")
    )
      continue;
    const staged = path.join(
      root,
      "assistant-runtime/resources/runtime",
      name.slice("assistant-runtime/".length),
    );
    assert.equal(
      hash(sourceBytes(name, await readFile(staged))),
      expected,
      "prepared runtime differs from the frozen source; prepare before running acceptance",
    );
  }
}

export async function main(argv = process.argv.slice(2)) {
  const options = {};
  for (let index = 0; index < argv.length; index++) {
    const flag = argv[index];
    if (["--live", "--replay-win10", "--verify", "--help"].includes(flag))
      options[flag.slice(2)] = true;
    else if (
      [
        "--out",
        "--platform",
        "--driver",
        "--apex",
        "--root-authorize-host",
      ].includes(flag)
    )
      options[flag.slice(2)] = argv[++index];
    else throw new Error(`unknown option ${flag}`);
  }
  if (options.help) {
    console.log(
      "node scripts/assistant-parity-eval.mjs --out DIR [--live --platform macos|win11 --root-authorize-host SAME --driver COMPILED_TEST_BINARY --apex ofox.ai|ofox.io]\nDefault: offline preparation only. Transfer the one authoritative campaign directory/ledger between hosts; never restart it or fork budget copies. Fixed allocations cap each paid host at USD5, each task USD1, campaign USD10. Native Win10 replay requires --replay-win10 --driver BINARY on the same Win11 capture campaign. No automatic build, model-task retry, images, or daily-account credential fallback.",
    );
    return 0;
  }
  assert.ok(options.out, "--out is required");
  const directory = path.resolve(options.out);
  if (!options.live && !options["replay-win10"] && !options.verify) {
    await prepare(directory, { apex: options.apex ?? "ofox.ai" });
    console.log(
      "Prepared twelve pending tasks and six pending Win10 replays offline; no key read or request sent.",
    );
    return 0;
  }
  const campaign = JSON.parse(
    await readFile(path.join(directory, "campaign.json"), "utf8"),
  );
  assert.equal(
    campaign.campaignHash,
    buildCampaign({
      sourceSha: campaign.sourceSha,
      gitSha: campaign.gitSha,
      generationFiles: campaign.generationFiles,
      apex: campaign.selectedApex,
    }).campaignHash,
    "campaign definition changed",
  );
  assert.equal(
    digest(await snapshot()),
    campaign.generationHash,
    "source snapshot changed; never mix capture code versions",
  );
  if (options.verify) {
    await report(directory, campaign);
    return 0;
  }
  assert.equal(
    execFileSync("git", ["rev-parse", "HEAD"], {
      cwd: root,
      encoding: "utf8",
    }).trim(),
    campaign.gitSha,
    "checkout HEAD differs from campaign",
  );
  assert.ok(
    execFileSync("git", ["status", "--porcelain", "--untracked-files=normal"], {
      cwd: root,
      encoding: "utf8",
    })
      .split(/\r?\n/)
      .every((line) => !line || line === "?? HANDOFF-codex.md"),
    "live/replay acceptance requires a clean committed checkout",
  );
  await verifyRuntimeSource(campaign);
  assert.ok(
    options.driver && path.isAbsolute(options.driver),
    "explicit compiled --driver path required",
  );
  if (options["replay-win10"]) {
    assert.equal(
      nativePlatform(),
      "win10",
      "replay requires native Windows 10, never ordinary Python or an output-only check",
    );
    return withLedgerLock(directory, async () => {
      const ledger = JSON.parse(
        await readFile(path.join(directory, "ledger.json"), "utf8"),
      );
      assert.equal(ledger.campaignHash, campaign.campaignHash);
      const proxy = {
        base: "http://localhost:1",
        token: `parity-proxy-${randomUUID()}-${randomUUID()}`,
      };
      for (const replay of campaign.win10Replays) {
        const task = campaign.tasks.find(
          (item) => item.id === replay.capturedTaskId,
        );
        const captured = JSON.parse(
          await readFile(
            path.join(directory, task.id, "verified-outcome.json"),
            "utf8",
          ),
        );
        captured.verifiedArtifact = await inspectArtifact(
          path.join(directory, task.id),
          task,
          captured,
          path.join(directory, "business-source.pptx"),
        );
        validateReceipt(campaign, task, captured, { manual: false });
        const { out, receipt } = await runDriver(
          options.driver,
          task,
          directory,
          campaign,
          proxy,
          "replay",
        );
        receipt.verifiedArtifact = await inspectArtifact(
          out,
          task,
          receipt,
          path.join(directory, "business-source.pptx"),
        );
        validateReceipt(campaign, task, receipt, { manual: false });
        assert.ok(
          receipt.mode === "replay" &&
            receipt.replayedCaptureSha256 === captured.captureSha256,
          "replay did not use the exact captured Win11 inputs",
        );
        await save(path.join(out, "replay-receipt.json"), {
          replayId: replay.id,
          capturedTaskId: task.id,
          capturedInputSha256: captured.captureSha256,
          accepted: true,
          manualAccepted: false,
          outcome: receipt,
        });
      }
      await report(directory, campaign);
      return 0;
    });
  }
  assert.ok(
    ["macos", "win11"].includes(options.platform) &&
      options.platform === nativePlatform(),
    "paid task must run on its native platform",
  );
  assert.equal(
    options["root-authorize-host"],
    options.platform,
    "root must explicitly approve this one host allocation before live use",
  );
  const apex = options.apex ?? campaign.selectedApex;
  assert.equal(apex, campaign.selectedApex, "apex is frozen for the campaign");
  return withLedgerLock(directory, async () => {
    const ledger = JSON.parse(
      await readFile(path.join(directory, "ledger.json"), "utf8"),
    );
    assert.equal(ledger.campaignHash, campaign.campaignHash);
    assert.ok(
      !ledger.stopped &&
        !ledger.hostApprovals.some(
          (item) => item.platform === options.platform,
        ),
      "host allocation already used or campaign stopped; no rerun",
    );
    const key = readKey();
    if (!key) {
      await save(path.join(directory, "pending.json"), {
        status: "pending",
        reason:
          "dedicated assistant-1701-eval / OFOX_API_KEY absent; no production-key fallback",
      });
      return 2;
    }
    const response = await fetch(`https://api.${apex}/v1/models`, {
      redirect: "error",
      headers: { Authorization: `Bearer ${key}` },
      signal: AbortSignal.timeout(30000),
    });
    assert.ok(response.ok, "catalog unavailable");
    const catalog = await response.json();
    for (const id of MODELS) {
      const row = catalog.data?.find((item) => item.id === id);
      assert.ok(
        row?.supported_endpoints?.includes("/v1/chat/completions"),
        `exact model unavailable: ${id}`,
      );
      price(row, "prompt");
      price(row, "completion");
    }
    await save(
      path.join(directory, `catalog-${options.platform}.json`),
      catalog,
    );
    ledger.hostApprovals.push({
      platform: options.platform,
      hostname: os.hostname(),
      osRelease: os.release(),
      architecture: process.arch,
      allocationUsd: "5.00",
      catalogSha256: digest(catalog),
      approvedAt: new Date().toISOString(),
      predecessorLedgerSha256: digest(ledger),
    });
    await save(path.join(directory, "ledger.json"), ledger);
    const proxy = await forwarder({
      campaign,
      ledger,
      directory,
      key,
      apex,
      catalog,
    });
    try {
      for (const task of campaign.tasks.filter(
        (item) => item.platform === options.platform,
      )) {
        assert.ok(
          !ledger.tasks[task.id] && !ledger.stopped,
          "task already attempted or campaign stopped",
        );
        ledger.tasks[task.id] = {
          status: "started",
          inputHash: task.inputHash,
        };
        await save(path.join(directory, "ledger.json"), ledger);
        proxy.select(task);
        const { out, receipt } = await runDriver(
          options.driver,
          task,
          directory,
          campaign,
          proxy,
        );
        receipt.verifiedArtifact = await inspectArtifact(
          out,
          task,
          receipt,
          path.join(directory, "business-source.pptx"),
        );
        await save(path.join(out, "verified-outcome.json"), receipt);
        validateReceipt(campaign, task, receipt, { manual: false });
        ledger.tasks[task.id] = {
          status: receipt.status,
          inputHash: task.inputHash,
        };
        await save(path.join(directory, "ledger.json"), ledger);
        assert.ok(
          receipt.status === "completed" && !ledger.stopped,
          "task failed; no additional paid task or retry is authorized by this harness",
        );
      }
    } finally {
      await proxy.close();
      for (const request of ledger.requests.filter(
        (item) => item.status === "reserved",
      ))
        settle(ledger, request.id, null);
      await save(path.join(directory, "ledger.json"), ledger);
      await report(directory, campaign);
    }
    return 0;
  });
}
if (
  process.argv[1] &&
  pathToFileURL(path.resolve(process.argv[1])).href === import.meta.url
)
  main()
    .then((code) => {
      process.exitCode = code;
    })
    .catch((error) => {
      console.error(redact(error.message));
      process.exitCode = 1;
    });
