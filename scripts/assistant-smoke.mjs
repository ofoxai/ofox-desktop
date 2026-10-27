import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { UNUSED_PACKAGE_PATHS } from "./assistant-prune.mjs";

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
  const { mkdtemp, readFile, rm, writeFile } = await import("node:fs/promises");
  const { tmpdir } = await import("node:os");
  const path = (await import("node:path")).default;
  const { pathToFileURL } = await import("node:url");
  const { createRequire } = await import("node:module");
  const production = process.argv[1];
  for (const name of ["pi-ai", "pi-coding-agent"]) {
    const manifest = path.join(
      production,
      "node_modules",
      "@earendil-works",
      name,
      "package.json",
    );
    assert.equal(JSON.parse(await readFile(manifest, "utf8")).version, "1.4.0");
  }
  // The native canvas addon loads in the bundled Node and encodes images.
  const require = createRequire(path.join(production, "package.json"));
  const { createCanvas } = await import(
    pathToFileURL(require.resolve("@napi-rs/canvas")).href
  );
  assert.equal(
    createCanvas(2, 2).toBuffer("image/png").subarray(1, 4).toString(),
    "PNG",
  );
  const { AssistantRuntime } = await import(
    pathToFileURL(path.join(production, "src/runtime.mjs")).href
  );
  const directory = await mkdtemp(
    path.join(tmpdir(), "ofox-packaged-runtime-"),
  );
  const testToken = "in-memory-production-smoke-credential";
  try {
    // The legacy PDF.js build reads text (CJK through the shipped CMaps) and
    // renders a page without text through the native canvas.
    const { AttachmentStore } = await import(
      pathToFileURL(path.join(production, "src/attachments.mjs")).href
    );
    const pages = [
      "BT /F1 12 Tf 20 150 Td <4E2D6587> Tj ET",
      "q 80 0 0 80 20 20 cm BI /W 1 /H 1 /CS /RGB /BPC 8 /F /AHx ID FF0000> EI Q",
    ];
    const objects = [
      "<< /Type /Catalog /Pages 2 0 R >>",
      "<< /Type /Pages /Kids [4 0 R 6 0 R] /Count 2 >>",
      "<< /Type /Font /Subtype /Type0 /BaseFont /STSong-Light /Encoding /UniGB-UCS2-H /DescendantFonts [8 0 R] >>",
      ...pages.flatMap((content, index) => [
        `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Resources << /Font << /F1 3 0 R >> >> /Contents ${5 + index * 2} 0 R >>`,
        `<< /Length ${content.length} >>\nstream\n${content}\nendstream`,
      ]),
      "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /STSong-Light /CIDSystemInfo << /Registry (Adobe) /Ordering (GB1) /Supplement 4 >> /FontDescriptor 9 0 R >>",
      "<< /Type /FontDescriptor /FontName /STSong-Light /Flags 6 /FontBBox [0 0 1000 1000] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 880 /StemV 80 >>",
    ];
    let pdf = "%PDF-1.4\n";
    const offsets = objects.map((object, index) => {
      const offset = pdf.length;
      pdf += `${index + 1} 0 obj\n${object}\nendobj\n`;
      return offset;
    });
    const xref = pdf.length;
    pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n${offsets.map((offset) => `${String(offset).padStart(10, "0")} 00000 n \n`).join("")}trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`;
    await writeFile(path.join(directory, "smoke-pdf"), pdf);
    const store = new AttachmentStore(directory, [
      { id: "pdf", name: "smoke.pdf", path: "smoke-pdf" },
    ]);
    const extracted = await store.read("pdf");
    assert.match(extracted.text, /中文/);
    assert.deepEqual(extracted.pagesWithoutText, [2]);
    const page = await store.image("pdf", 2);
    assert.equal(
      Buffer.from(page.data, "base64").subarray(1, 4).toString(),
      "PNG",
    );

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
      prompt: "hello",
      apiKey: testToken,
      apex: "ofox.ai",
      model: {
        id: "test/packaged",
        supportedEndpoints: ["/v1/chat/completions"],
        contextWindow: 32000,
        maxOutputTokens: 1024,
      },
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

    // The copied search server bundle runs without node_modules, serves
    // exactly its two tools to the shipped pi-mcp client, and refuses to
    // start without its configuration.
    const { spawnSync } = await import("node:child_process");
    const bundle = path.join(
      production,
      "..",
      "search-mcp",
      "ofox-search-mcp.mjs",
    );
    const refused = spawnSync(process.execPath, [bundle], {
      env: {},
      input: "",
      encoding: "utf8",
    });
    assert.equal(refused.status, 2);
    assert.equal(refused.stdout, "");
    assert.match(refused.stderr, /^ofox-search-mcp: OFOX_ASSISTANT_KEY /);
    const { McpClient, StdioTransport } = await import(
      pathToFileURL(
        path.join(
          production,
          "node_modules/@earendil-works/pi-mcp/dist/index.js",
        ),
      ).href
    );
    const client = new McpClient({ name: "smoke", version: "0.0.0" });
    await client.connect(
      new StdioTransport({
        command: process.execPath,
        args: [bundle],
        env: {
          OFOX_ASSISTANT_KEY: testToken,
          OFOX_BASE_URL: "https://api.ofox.ai",
        },
        inheritEnv: false,
      }),
    );
    assert.deepEqual(
      (await client.listTools()).map((tool) => tool.name).sort(),
      ["web_fetch", "web_search"],
    );
    await client.close();

    // A web search through the packaged runtime: the runtime finds the
    // bundle next to itself and the bundle reaches a local gateway.
    const { createServer } = await import("node:http");
    const searches = [];
    const gateway = createServer((request, response) => {
      searches.push(request.headers.authorization);
      response.writeHead(200, { "content-type": "application/json" });
      response.end(
        JSON.stringify({
          model: "openai/gpt-5-nano",
          output: [
            {
              type: "web_search_call",
              action: {
                sources: [{ type: "url", url: "https://example.com/smoke" }],
              },
            },
            {
              type: "message",
              content: [{ type: "output_text", text: "Smoke summary." }],
            },
          ],
          usage: { input_tokens: 10, output_tokens: 5 },
        }),
      );
    });
    await new Promise((resolve) => gateway.listen(0, "localhost", resolve));
    const baseUrl = `http://localhost:${gateway.address().port}`;
    let turns = 0;
    const searching = new AssistantRuntime({
      fetch: async () => {
        turns++;
        const delta =
          turns === 1
            ? {
                tool_calls: [
                  {
                    index: 0,
                    id: "smoke-search",
                    type: "function",
                    function: {
                      name: "web_search",
                      arguments: JSON.stringify({ query: "smoke" }),
                    },
                  },
                ],
              }
            : { content: "Searched." };
        return new Response(
          [
            { choices: [{ index: 0, delta: { role: "assistant", ...delta } }] },
            {
              choices: [
                {
                  index: 0,
                  delta: {},
                  finish_reason: turns === 1 ? "tool_calls" : "stop",
                },
              ],
            },
          ]
            .map((value) => `data: ${JSON.stringify(value)}\n\n`)
            .join("") + "data: [DONE]\n\n",
          { headers: { "content-type": "text/event-stream" } },
        );
      },
    });
    const searched = await searching.run({
      sessionId: "smoke-search-session",
      runId: "smoke-search-run",
      operationId: "smoke-search-operation",
      sessionFile: path.join(directory, "search.jsonl"),
      prompt: "search",
      apiKey: testToken,
      apex: "ofox.ai",
      baseUrl,
      allowLocalGateway: true,
      webSearch: true,
      model: {
        id: "test/packaged",
        supportedEndpoints: ["/v1/chat/completions"],
        contextWindow: 32000,
        maxOutputTokens: 1024,
      },
    });
    await searching.close();
    await new Promise((resolve) => gateway.close(resolve));
    assert.equal(searched.status, "completed");
    assert.deepEqual(searches, [`Bearer ${testToken}`]);
    const tool = searched.entries.find((entry) => entry.role === "tool");
    assert.equal(tool.toolName, "web_search");
    assert.equal(tool.details.usage.searchCalls, 1);
    assert.deepEqual(tool.details.sources, [
      { title: "example.com", url: "https://example.com/smoke" },
    ]);
    process.stdout.write(
      `Production smoke passed (${process.version}; Pi SDK, native canvas, PDF text and rendering, native JSONL, search server)\n`,
    );
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
}

// The prepared runtime must not claim a different macOS floor than the app.
const readJson = (file) => JSON.parse(readFileSync(file, "utf8"));
assert.equal(
  readJson(path.join(resources, "manifest.json")).minimumMacOS,
  readJson(path.join(root, "src-tauri", "tauri.conf.json")).bundle?.macOS
    ?.minimumSystemVersion,
  "resources/manifest.json minimumMacOS must equal tauri.conf.json bundle.macOS.minimumSystemVersion; run pnpm assistant:prepare",
);

// Pruned package paths stay out of the bundle; the PDF.js legacy build, its
// resources and the third-party notices stay in it.
const production = path.join(resources, "runtime");
for (const unused of UNUSED_PACKAGE_PATHS)
  assert.equal(
    existsSync(path.join(production, "node_modules", unused)),
    false,
    `${unused} must be pruned; run pnpm assistant:prepare`,
  );
for (const kept of [
  "node_modules/pdfjs-dist/legacy/build/pdf.mjs",
  "node_modules/pdfjs-dist/legacy/build/pdf.worker.mjs",
  "node_modules/pdfjs-dist/cmaps",
  "node_modules/pdfjs-dist/iccs",
  "node_modules/pdfjs-dist/wasm",
  "node_modules/pdfjs-dist/LICENSE",
])
  assert.ok(existsSync(path.join(production, kept)), `${kept} must ship`);
for (const office of ["fflate", "fast-xml-parser"])
  assert.equal(
    existsSync(path.join(production, "node_modules", office)),
    false,
    `${office} must not ship`,
  );
assert.ok(
  existsSync(path.join(resources, "search-mcp", "ofox-search-mcp.mjs")),
  "search-mcp/ofox-search-mcp.mjs must ship; run pnpm assistant:prepare",
);
for (const notice of [
  "napi-rs-canvas.txt",
  "pdfjs-dist.txt",
  "ofox-search-mcp.txt",
])
  assert.ok(
    existsSync(path.join(resources, "notices", notice)),
    `notices/${notice} must ship; run pnpm assistant:prepare`,
  );

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
