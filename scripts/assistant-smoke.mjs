import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const resources = path.join(root, "assistant-runtime/resources");
const node = path.join(
  resources,
  "node",
  process.platform === "win32" ? "node.exe" : "node",
);
const environment = Object.fromEntries(
  ["PATH", "SystemRoot", "TEMP", "TMP"]
    .filter((key) => process.env[key])
    .map((key) => [key, process.env[key]]),
);

// Run the actual copied production tree with no user credentials or SDK config.
async function productionSmoke() {
  const assert = (await import("node:assert/strict")).default;
  const { mkdtemp, readFile, rm } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const path = (await import("node:path")).default;
  const { pathToFileURL } = await import("node:url");
  const { createRequire } = await import("node:module");
  const production = process.argv[1];
  const require = createRequire(path.join(production, "package.json"));
  const { createCanvas } = await import(
    pathToFileURL(require.resolve("@napi-rs/canvas")).href
  );
  assert.equal(
    createCanvas(2, 2).toBuffer("image/png").subarray(1, 4).toString(),
    "PNG",
  );
  await import(
    pathToFileURL(require.resolve("pdfjs-dist/legacy/build/pdf.mjs")).href
  );
  const { AssistantRuntime } = await import(
    pathToFileURL(path.join(production, "src/runtime.mjs")).href
  );
  const directory = await mkdtemp(
    path.join(tmpdir(), "ofox-packaged-runtime-"),
  );
  const testToken = "in-memory-production-smoke-credential";
  try {
    const runtime = new AssistantRuntime({
      fetch: async (url, init) => {
        assert.equal(String(url), "https://api.ofox.ai/v1/chat/completions");
        assert.equal(
          new Headers(init.headers).get("authorization"),
          `Bearer ${testToken}`,
        );
        const chunks = [
          {
            choices: [
              {
                index: 0,
                delta: { role: "assistant", content: "Packaged runtime ready" },
                finish_reason: null,
              },
            ],
          },
          { choices: [{ index: 0, delta: {}, finish_reason: "stop" }] },
        ];
        return new Response(
          chunks.map((value) => `data: ${JSON.stringify(value)}\n\n`).join("") +
            "data: [DONE]\n\n",
          { headers: { "content-type": "text/event-stream" } },
        );
      },
    });
    const result = await runtime.run({
      sessionId: "smoke-session",
      runId: "smoke-run",
      operationId: "smoke-operation",
      sessionFile: path.join(directory, "session.jsonl"),
      kind: "general",
      prompt: "hello",
      accessToken: testToken,
      apex: "ofox.ai",
      model: {
        id: "test/packaged",
        supportedEndpoints: ["/v1/chat/completions"],
        contextWindow: 32000,
        maxOutputTokens: 1024,
      },
      attachmentRoot: directory,
      attachments: [],
    });
    assert.equal(result.status, "completed");
    assert.equal(result.entries.at(-1).content, "Packaged runtime ready");
    assert.equal(
      (await readFile(result.sessionFile, "utf8")).includes(testToken),
      false,
    );
    assert.deepEqual(
      (await runtime.load({ sessionFile: result.sessionFile })).entries,
      result.entries,
    );
    await runtime.close();
    process.stdout.write(
      `Production smoke passed (${process.version}; SDK, native canvas, PDF parser, native JSONL)\n`,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

const result = spawnSync(
  node,
  [
    "--input-type=module",
    "--eval",
    `(${productionSmoke.toString()})().catch(error => { console.error(error); process.exitCode = 1; });`,
    path.join(resources, "runtime"),
  ],
  { env: environment, stdio: "inherit" },
);
process.exit(result.status ?? 1);
