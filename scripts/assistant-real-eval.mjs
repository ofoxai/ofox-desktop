#!/usr/bin/env node
// assistant-real-eval.mjs — 真实模型回归（最小版）：四个验收提示 × 选定的模型，
// 走真实网关，写一份 Markdown 报告。会花真钱。
//
// 用法：
//   pnpm assistant:real-eval [--apex ofox.ai] [--account assistant-key]
//                            [--models terra,luna,deepseek-v4.1-flash] [--out DIR]
//   node scripts/assistant-real-eval.mjs --self-test   # 不联网、不跑 cargo、不需要 key
//
// 步骤：
//   1. key：环境变量 OFOX_API_KEY；没有就读钥匙串
//      `security find-generic-password -s ai.ofox.desktop.apikey -a <account> -w`。
//   2. GET https://api.<apex>/v1/models，每个模式取 id 含它的第一个模型；
//      有模式没选中就退出码 2，并列出目录里的全部 id。
//   3. 跑 `cargo test --test assistant_host real_eval -- --ignored`（src-tauri 里的
//      `real_eval::four_prompts_on_the_listed_models`），每个用例在 stdout 打印一行 JSON。
//   4. 收集这些 JSON 行，写 <out>/report.md；用例生成的文件在
//      <out>/<模型>/<用例>/ 下。
//
// 退出码：0 失败 ≤ 1 个；1 失败 ≥ 2 个；2 用法或环境错误（没有 key、目录读不到、
// 模型没选中）。
//
// key 只经子进程环境变量 OFOX_REAL_EVAL_KEY 交给 cargo，不进命令行参数；脚本不打印
// key，转发的 cargo 输出和 report.md 都会把 key、"Bearer …" 和形似 key 的串遮成 ***。

import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import { mkdirSync, writeFileSync } from "node:fs";
import path from "node:path";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const SERVICE = "ai.ofox.desktop.apikey";
const APEXES = ["ofox.ai", "ofox.io"];
const DEFAULTS = {
  apex: "ofox.ai",
  account: "assistant-key",
  models: "terra,luna,deepseek-v4.1-flash",
};
const CASES_PER_MODEL = 4;
const CARGO_ARGS = [
  "test",
  "--manifest-path",
  "src-tauri/Cargo.toml",
  "--test",
  "assistant_host",
  "real_eval",
  "--",
  "--ignored",
  "--nocapture",
  "--test-threads=1",
];
const REDACTED = "***";

const HELP = `真实模型回归：四个验收提示（pdf / ppt / 成绩表 / Word 周报）× 选定的模型，走真实网关。会花真钱。

用法：
  pnpm assistant:real-eval [选项]
  node scripts/assistant-real-eval.mjs [选项]

选项：
  --apex <ofox.ai|ofox.io>  网关区域，默认 ${DEFAULTS.apex}（请求发往 https://api.<apex>）
  --account <名字>          钥匙串 account，默认 ${DEFAULTS.account}（也可以是工具 slug，如 codex）
  --models <模式,…>         逗号分隔的模型 id 模式，每个取 id 含它的第一个模型，
                            默认 ${DEFAULTS.models}
  --out <目录>              报告和文件的目录，默认 .real-eval/<ISO 时间>
  --self-test               只自检脚本本身：不联网、不跑 cargo、不需要 key
  -h, --help                显示本帮助

key：环境变量 OFOX_API_KEY；没有就读钥匙串 service ${SERVICE} / account <--account>（仅 macOS）。

输出：<out>/report.md（每个用例一行，末尾 "通过 N / 总数"），以及 <out>/<模型>/<用例>/ 下
用例生成的文件、它们的文字（.txt）、渲染页（.pages/）和回复（reply.md）。

退出码：0 失败 ≤ 1 个；1 失败 ≥ 2 个；2 用法或环境错误。`;

class UsageError extends Error {}

function parseArgs(argv) {
  const options = {
    ...DEFAULTS,
    out: null,
    help: false,
    selfTest: false,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index];
    if (arg === "-h" || arg === "--help") {
      options.help = true;
      continue;
    }
    if (arg === "--self-test") {
      options.selfTest = true;
      continue;
    }
    const match = /^--(apex|account|models|out)(?:=(.*))?$/.exec(arg);
    if (!match) throw new UsageError(`未知参数：${arg}`);
    let value = match[2];
    if (value === undefined) {
      value = argv[index + 1];
      if (value === undefined || value.startsWith("--")) value = "";
      else index += 1;
    }
    if (!value) throw new UsageError(`--${match[1]} 需要一个值`);
    options[match[1]] = value;
  }
  if (!APEXES.includes(options.apex))
    throw new UsageError(
      `--apex 只能是 ${APEXES.join(" 或 ")}，不是 ${options.apex}`,
    );
  options.patterns = options.models
    .split(",")
    .map((pattern) => pattern.trim())
    .filter(Boolean);
  if (!options.patterns.length)
    throw new UsageError("--models 至少要有一个模式");
  return options;
}

// `.real-eval/2026-11-08T03-12-45Z`：ISO 时间，冒号换成 -，Windows 也能当目录名。
function defaultOut(now = new Date()) {
  const stamp = now
    .toISOString()
    .replace(/\.\d+Z$/, "Z")
    .replace(/:/g, "-");
  return path.join(root, ".real-eval", stamp);
}

function today(now = new Date()) {
  const pad = (number) => String(number).padStart(2, "0");
  return `${now.getFullYear()}-${pad(now.getMonth() + 1)}-${pad(now.getDate())}`;
}

// 先看 OFOX_API_KEY，再读 macOS 钥匙串。读不到（没有条目、拒绝授权、不是 macOS）
// 都返回空 key，由调用方给出说明并以 2 退出。
function readKey(
  account,
  { env = process.env, platform = process.platform, exec = execFileSync } = {},
) {
  const fromEnv = env.OFOX_API_KEY?.trim();
  if (fromEnv) return { key: fromEnv, source: "环境变量 OFOX_API_KEY" };
  if (platform !== "darwin") return { key: "", source: null };
  try {
    const key = exec(
      "security",
      ["find-generic-password", "-s", SERVICE, "-a", account, "-w"],
      { encoding: "utf8", stdio: ["ignore", "pipe", "ignore"] },
    ).trim();
    return { key, source: key ? `钥匙串 ${SERVICE} / ${account}` : null };
  } catch {
    return { key: "", source: null };
  }
}

// 每个模式取 id 含它的第一行；两个模式选中同一行只跑一次。
function pickModels(catalog, patterns) {
  const rows = Array.isArray(catalog?.data)
    ? catalog.data.filter((row) => typeof row?.id === "string")
    : [];
  const picked = [];
  const missing = [];
  for (const pattern of patterns) {
    const row = rows.find((candidate) => candidate.id.includes(pattern));
    if (!row) missing.push(pattern);
    else if (!picked.includes(row)) picked.push(row);
  }
  return { picked, missing, ids: rows.map((row) => row.id).sort() };
}

async function fetchCatalog(apex, key, fetchImpl = fetch) {
  const url = `https://api.${apex}/v1/models`;
  const response = await fetchImpl(url, {
    headers: { Authorization: `Bearer ${key}` },
  });
  if (!response.ok) throw new Error(`GET ${url} 返回 HTTP ${response.status}`);
  return response.json();
}

// 已知的 key，以及任何形似凭据的串（同 assistant-runtime/src/security.mjs 的 redact）。
function mask(text, key = "") {
  let result = String(text);
  if (key) result = result.split(key).join(REDACTED);
  return result
    .replace(/\b([a-z][a-z0-9+.-]*:\/\/)[^\s/@]+@/gi, `$1${REDACTED}@`)
    .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+/=-]+/gi, `$1 ${REDACTED}`)
    .replace(/\bsk-(?:of-|ofi-|ant-|proj-)?[A-Za-z0-9_-]{8,}/g, REDACTED)
    .replace(
      /((?:api[_-]?key|access[_-]?token|refresh[_-]?token|password|secret)\s*[=:]\s*)(["']?)[^\s,"'}]+/gi,
      `$1${REDACTED}`,
    );
}

// cargo 的 stdout 里夹着测试框架的输出；只认带 case 和 ok 的 JSON 对象。串行运行时
// libtest 先打印不换行的 "test <名字> ... "，第一行结果可能接在它后面，所以从 {"case"
// 处读起。
function parseLine(line) {
  const start = line.indexOf('{"case"');
  const text = (start >= 0 ? line.slice(start) : line).trim();
  if (!text.startsWith("{")) return null;
  try {
    const value = JSON.parse(text);
    return typeof value?.case === "string" && typeof value?.ok === "boolean"
      ? value
      : null;
  } catch {
    return null;
  }
}

function cargoEnv({ key, apex, models, out }, env = process.env) {
  const child = {
    ...env,
    OFOX_REAL_EVAL_KEY: key,
    OFOX_REAL_EVAL_APEX: apex,
    OFOX_REAL_EVAL_MODELS: JSON.stringify(models),
    OFOX_REAL_EVAL_OUT: out,
  };
  // key 只走 OFOX_REAL_EVAL_KEY 这一条路。
  delete child.OFOX_API_KEY;
  return child;
}

function runCargo(env, key) {
  return new Promise((resolve) => {
    const lines = [];
    const child = spawn("cargo", CARGO_ARGS, {
      cwd: root,
      env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    createInterface({ input: child.stdout }).on("line", (line) => {
      const parsed = parseLine(line);
      if (parsed) lines.push(parsed);
      console.log(mask(line, key));
    });
    createInterface({ input: child.stderr }).on("line", (line) =>
      console.error(mask(line, key)),
    );
    child.on("error", (error) => {
      console.error(`无法启动 cargo：${error.message}`);
      resolve({ code: null, lines });
    });
    child.on("close", (code) => resolve({ code, lines }));
  });
}

function cell(value) {
  return String(value ?? "")
    .replace(/\r?\n/g, " ")
    .replace(/\|/g, "\\|");
}

function verdict(line) {
  const note = cell(line.note);
  if (line.ok) return note ? `通过（${note}）` : "通过";
  return note ? `失败：${note}` : "失败";
}

function passedOf(lines) {
  return lines.filter((line) => line.ok === true).length;
}

function renderReport(lines, { date, apex, models, total, cargoCode }) {
  const rows = lines.map((line) => {
    const tokens = `${line.tokens?.input ?? 0} / ${line.tokens?.output ?? 0}`;
    return `| ${date} | ${cell(line.model)} | ${cell(line.case)} | ${line.files ?? 0} | ${line.warnings ?? 0} | ${line.fixed ?? 0} | ${line.calls ?? 0} | ${line.seconds ?? 0} | ${tokens} | ${verdict(line)} |`;
  });
  const out = [
    `# 真实模型回归 ${date}`,
    "",
    `网关：https://api.${apex}；模型：${models.map(cell).join("、")}`,
    "",
    "| 日期 | 模型 | 用例 | 文件数 | 警告 | 自动修正 | 调用次数 | 秒 | tokens | 结果 |",
    "| --- | --- | --- | ---: | ---: | ---: | ---: | ---: | --- | --- |",
    ...rows,
    "",
  ];
  if (lines.length < total)
    out.push(
      `缺 ${total - lines.length} 行结果（cargo 退出码 ${cargoCode ?? "无"}），按失败计。`,
      "",
    );
  out.push(`通过 ${passedOf(lines)} / ${total}`, "");
  return out.join("\n");
}

// 没跑出来的用例也算失败。
function exitCodeFor(lines, total) {
  return total - passedOf(lines) <= 1 ? 0 : 1;
}

async function main(argv) {
  let options;
  try {
    options = parseArgs(argv);
  } catch (error) {
    if (!(error instanceof UsageError)) throw error;
    console.error(
      `${error.message}\n用法见 node scripts/assistant-real-eval.mjs --help`,
    );
    return 2;
  }
  if (options.help) {
    console.log(HELP);
    return 0;
  }
  if (options.selfTest) return selfTest();

  const { key, source } = readKey(options.account);
  if (!key) {
    console.error(
      [
        "没有找到 Ofox API key，什么都没跑。",
        `  · 设置环境变量 OFOX_API_KEY，或`,
        `  · 在 macOS 钥匙串里放一条 service ${SERVICE} / account ${options.account}（Ofox Desktop`,
        "    正式版的助手发过一次消息后就有；工具 key 用 --account <工具 slug>）。",
        "  钥匙串弹出授权框时请选择允许。",
      ].join("\n"),
    );
    return 2;
  }
  activeKey = key;
  console.log(`key：来自${source}（不显示）`);

  let catalog;
  try {
    catalog = await fetchCatalog(options.apex, key);
  } catch (error) {
    console.error(mask(`读不到模型目录：${error.message}`, key));
    return 2;
  }
  const { picked, missing, ids } = pickModels(catalog, options.patterns);
  if (missing.length) {
    console.error(
      `这些模式没有选中模型：${missing.join(", ")}\n目录里的模型（${ids.length} 个）：\n  ${ids.join("\n  ")}`,
    );
    return 2;
  }
  const out = options.out ? path.resolve(options.out) : defaultOut();
  mkdirSync(out, { recursive: true });
  const models = picked.map((row) => row.id);
  const total = picked.length * CASES_PER_MODEL;
  console.log(`模型：${models.join("、")}（${total} 个用例）`);
  console.log(`输出：${out}`);

  const { code, lines } = await runCargo(
    cargoEnv({ key, apex: options.apex, models: picked, out }),
    key,
  );
  const report = path.join(out, "report.md");
  writeFileSync(
    report,
    mask(
      renderReport(lines, {
        date: today(),
        apex: options.apex,
        models,
        total,
        cargoCode: code,
      }),
      key,
    ),
  );
  console.log(`\n通过 ${passedOf(lines)} / ${total}；报告：${report}`);
  return exitCodeFor(lines, total);
}

// ── 自检：不联网、不跑 cargo、不需要 key ─────────────────────────────────

function selfTest() {
  const checks = [];
  const check = (name, body) => checks.push([name, body]);
  const fakeKey = "sk-of-selftest-0123456789abcdef";

  check("默认选项", () => {
    const options = parseArgs([]);
    assert.equal(options.apex, "ofox.ai");
    assert.equal(options.account, "assistant-key");
    assert.deepEqual(options.patterns, [
      "terra",
      "luna",
      "deepseek-v4.1-flash",
    ]);
    assert.equal(options.out, null);
  });
  check("选项两种写法", () => {
    const options = parseArgs([
      "--apex=ofox.io",
      "--account",
      "codex",
      "--models",
      " a, b ,,",
      "--out",
      "x/y",
    ]);
    assert.equal(options.apex, "ofox.io");
    assert.equal(options.account, "codex");
    assert.deepEqual(options.patterns, ["a", "b"]);
    assert.equal(options.out, "x/y");
    assert.equal(parseArgs(["--help"]).help, true);
    assert.equal(parseArgs(["--self-test"]).selfTest, true);
  });
  check("错误的选项是用法错误", () => {
    for (const argv of [
      ["--apex", "example.com"],
      ["--models"],
      ["--models", "--out", "x"],
      ["--models", " , "],
      ["--bogus"],
      ["positional"],
    ])
      assert.throws(() => parseArgs(argv), UsageError, argv.join(" "));
  });
  check("默认输出目录", () => {
    const out = defaultOut(new Date("2026-11-08T03:12:45.678Z"));
    assert.equal(out, path.join(root, ".real-eval", "2026-11-08T03-12-45Z"));
    assert.equal(today(new Date(2026, 10, 8, 23, 59)), "2026-11-08");
  });
  check("key：环境变量优先，其次钥匙串", () => {
    const never = () => assert.fail("不该读钥匙串");
    assert.deepEqual(
      readKey("assistant-key", {
        env: { OFOX_API_KEY: ` ${fakeKey}\n` },
        platform: "darwin",
        exec: never,
      }),
      { key: fakeKey, source: "环境变量 OFOX_API_KEY" },
    );
    let asked;
    const found = readKey("codex", {
      env: { OFOX_API_KEY: "" },
      platform: "darwin",
      exec: (file, args, options) => {
        asked = { file, args, options };
        return `${fakeKey}\n`;
      },
    });
    assert.equal(found.key, fakeKey);
    assert.equal(asked.file, "security");
    assert.deepEqual(asked.args, [
      "find-generic-password",
      "-s",
      "ai.ofox.desktop.apikey",
      "-a",
      "codex",
      "-w",
    ]);
    assert.deepEqual(asked.options.stdio, ["ignore", "pipe", "ignore"]);
    const missing = readKey("assistant-key", {
      env: {},
      platform: "darwin",
      exec: () => {
        throw new Error(
          "The specified item could not be found in the keychain.",
        );
      },
    });
    assert.deepEqual(missing, { key: "", source: null });
    assert.deepEqual(
      readKey("assistant-key", { env: {}, platform: "win32", exec: never }),
      { key: "", source: null },
    );
  });
  check("模式匹配 /v1/models", () => {
    const catalog = {
      object: "list",
      data: [
        { id: "openai/gpt-image-2" },
        { id: "openai/gpt-6.1-terra", supported_endpoints: ["/v1/responses"] },
        { id: "openai/gpt-6.1-terra-mini" },
        { id: "google/gemini-luna" },
        { id: "deepseek/deepseek-v4.1-flash" },
        { name: "no id" },
      ],
    };
    const { picked, missing, ids } = pickModels(catalog, [
      "terra",
      "luna",
      "deepseek-v4.1-flash",
    ]);
    assert.deepEqual(
      picked.map((row) => row.id),
      [
        "openai/gpt-6.1-terra",
        "google/gemini-luna",
        "deepseek/deepseek-v4.1-flash",
      ],
    );
    assert.deepEqual(
      picked[0].supported_endpoints,
      ["/v1/responses"],
      "原样交给测试",
    );
    assert.deepEqual(missing, []);
    assert.equal(ids.length, 5);
    const again = pickModels(catalog, ["gpt-6.1", "terra", "nope"]);
    assert.deepEqual(
      again.picked.map((row) => row.id),
      ["openai/gpt-6.1-terra"],
      "同一行只跑一次",
    );
    assert.deepEqual(again.missing, ["nope"]);
    assert.deepEqual(pickModels({}, ["terra"]).missing, ["terra"]);
  });
  check("cargo 的参数与环境", () => {
    assert.deepEqual(CARGO_ARGS, [
      "test",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--test",
      "assistant_host",
      "real_eval",
      "--",
      "--ignored",
      "--nocapture",
      "--test-threads=1",
    ]);
    assert.ok(!CARGO_ARGS.join(" ").includes(fakeKey));
    const models = [{ id: "openai/gpt-6.1-terra" }];
    const env = cargoEnv(
      { key: fakeKey, apex: "ofox.io", models, out: "/tmp/out" },
      { PATH: "/usr/bin", OFOX_API_KEY: fakeKey },
    );
    assert.equal(env.PATH, "/usr/bin");
    assert.equal(env.OFOX_REAL_EVAL_KEY, fakeKey);
    assert.equal(env.OFOX_REAL_EVAL_APEX, "ofox.io");
    assert.deepEqual(JSON.parse(env.OFOX_REAL_EVAL_MODELS), models);
    assert.equal(env.OFOX_REAL_EVAL_OUT, "/tmp/out");
    assert.equal(env.OFOX_API_KEY, undefined);
  });
  const passLine = (model, name) => ({
    case: name,
    model,
    ok: true,
    files: 1,
    warnings: 0,
    fixed: 1,
    svgPages: 8,
    calls: 1,
    seconds: 42.5,
    tokens: { input: 1200, output: 3400 },
    note: "",
  });
  const cases = [
    "pdf-sichuan",
    "pptx-transformer",
    "xlsx-grades",
    "docx-weekly",
  ];
  const results = [
    "openai/gpt-6.1-terra",
    "google/gemini-luna",
    "deepseek/deepseek-v4.1-flash",
  ].flatMap((model) =>
    cases.map((name) => JSON.stringify(passLine(model, name))),
  );
  // libtest 串行运行（--test-threads=1）时的真实排布："test <名字> ... " 不换行，
  // 第一行结果接在它后面，"ok" 在最后一行结果之后。
  const libtest = (lines) => [
    "{not json",
    '{"other": true}',
    "running 1 test",
    `test real_eval::four_prompts_on_the_listed_models ... ${lines[0]}`,
    ...lines.slice(1),
    "ok",
    "",
    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 101 filtered out; finished in 812.40s",
  ];
  const stdout = libtest(results);
  check("只收集用例的 JSON 行（第一行接在 libtest 的前缀后）", () => {
    const lines = stdout.map(parseLine).filter(Boolean);
    assert.equal(lines.length, 12);
    assert.equal(lines[0].case, "pdf-sichuan");
    assert.equal(lines[0].model, "openai/gpt-6.1-terra");
    assert.equal(lines[11].model, "deepseek/deepseek-v4.1-flash");
  });
  check("只收集用例的 JSON 行（测试先打印一个空行）", () => {
    const lines = libtest(["", ...results])
      .map(parseLine)
      .filter(Boolean);
    assert.equal(lines.length, 12);
    assert.equal(lines[0].case, "pdf-sichuan");
    assert.equal(parseLine('{"ok": false, "case": "x"}')?.case, "x");
    assert.equal(parseLine('test a ... {"case": "x"}'), null, "没有 ok");
  });
  check("报告：表头、行、合计", () => {
    const lines = stdout.map(parseLine).filter(Boolean);
    lines[1] = {
      ...lines[1],
      ok: false,
      warnings: 2,
      note: `run "error"; error x: Bearer ${fakeKey} | a\nb`,
    };
    const report = mask(
      renderReport(lines, {
        date: "2026-11-08",
        apex: "ofox.ai",
        models: ["openai/gpt-6.1-terra"],
        total: 12,
        cargoCode: 0,
      }),
      fakeKey,
    );
    const rows = report.split("\n");
    assert.ok(
      rows.includes(
        "| 日期 | 模型 | 用例 | 文件数 | 警告 | 自动修正 | 调用次数 | 秒 | tokens | 结果 |",
      ),
    );
    assert.equal(
      rows.filter((row) => row.startsWith("| 2026-11-08 |")).length,
      12,
    );
    assert.ok(
      rows.includes(
        "| 2026-11-08 | openai/gpt-6.1-terra | pdf-sichuan | 1 | 0 | 1 | 1 | 42.5 | 1200 / 3400 | 通过 |",
      ),
    );
    const failed = rows.find(
      (row) => row.includes("pptx-transformer") && row.includes("失败"),
    );
    assert.ok(failed.includes("\\| a b"), failed);
    assert.ok(report.trimEnd().endsWith("通过 11 / 12"));
    assert.ok(!report.includes(fakeKey));
    assert.ok(!report.includes("selftest"));
  });
  check("报告：缺行按失败计", () => {
    const lines = stdout.map(parseLine).filter(Boolean).slice(0, 10);
    const report = renderReport(lines, {
      date: "2026-11-08",
      apex: "ofox.io",
      models: ["a"],
      total: 12,
      cargoCode: 101,
    });
    assert.ok(report.includes("缺 2 行结果（cargo 退出码 101）"));
    assert.ok(report.trimEnd().endsWith("通过 10 / 12"));
    assert.equal(exitCodeFor(lines, 12), 1);
  });
  check("退出码：失败 ≤ 1 为 0", () => {
    const lines = stdout.map(parseLine).filter(Boolean);
    assert.equal(exitCodeFor(lines, 12), 0);
    lines[0] = { ...lines[0], ok: false };
    assert.equal(exitCodeFor(lines, 12), 0);
    lines[1] = { ...lines[1], ok: false };
    assert.equal(exitCodeFor(lines, 12), 1);
    assert.equal(exitCodeFor([], 12), 1);
  });
  check("遮住 key 和形似凭据的串", () => {
    const text = mask(
      `key ${fakeKey} Authorization: Bearer abc.def-123 api_key=xyz sk-ant-abcdefgh12 https://u:p@host/x`,
      fakeKey,
    );
    assert.ok(!text.includes(fakeKey));
    assert.ok(!text.includes("abc.def-123"));
    assert.ok(!text.includes("xyz"));
    assert.ok(!text.includes("sk-ant-abcdefgh12"));
    assert.ok(!text.includes("u:p@"));
    assert.ok(text.includes("Bearer ***"));
    assert.equal(mask("plain text", ""), "plain text");
  });

  let failed = 0;
  for (const [name, body] of checks) {
    try {
      body();
      console.log(`ok   ${name}`);
    } catch (error) {
      failed += 1;
      console.log(
        `FAIL ${name}\n     ${String(error?.message ?? error)
          .split("\n")
          .join("\n     ")}`,
      );
    }
  }
  console.log(
    `\nself-test: ${checks.length - failed} passed, ${failed} failed`,
  );
  return failed ? 1 : 0;
}

// 顶层出错时也遮住已经读到的 key。
let activeKey = "";

main(process.argv.slice(2)).then(
  (code) => {
    process.exitCode = code;
  },
  (error) => {
    console.error(mask(error?.stack ?? String(error), activeKey));
    process.exitCode = 2;
  },
);
