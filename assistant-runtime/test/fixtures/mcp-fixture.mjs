import readline from "node:readline";
import { spawn } from "node:child_process";
import { appendFileSync, writeFileSync } from "node:fs";
if (process.env.MCP_CHILD_FILE) {
  const child = spawn(
    process.execPath,
    [
      "-e",
      "process.on('SIGTERM',()=>{});setInterval(()=>{},1000);process.send('ready')",
    ],
    { stdio: ["ignore", "ignore", "ignore", "ipc"] },
  );
  await new Promise((resolve) => child.once("message", resolve));
  child.disconnect();
  writeFileSync(process.env.MCP_CHILD_FILE, String(child.pid));
}
if (process.env.MCP_PID_FILE)
  writeFileSync(process.env.MCP_PID_FILE, String(process.pid));
const send = (data) =>
  process.stdout.write(JSON.stringify({ jsonrpc: "2.0", ...data }) + "\n");
for await (const line of readline.createInterface({ input: process.stdin })) {
  const q = JSON.parse(line);
  if (q.id === undefined) continue;
  if (q.method === "initialize" && process.env.MCP_SCENARIO === "secrets") {
    const bytes = Buffer.from((process.env.SHORT_SECRET + " ").repeat(30000));
    for (let i = 0; i < bytes.length; i += 997)
      process.stderr.write(bytes.subarray(i, i + 997));
    send({
      method: "notifications/message",
      params: {
        level: "info",
        data:
          "Readable notification " +
          process.env.API_KEY +
          " " +
          process.env.SHORT_SECRET,
      },
    });
  }
  if (q.method === "initialize" && process.env.MCP_SCENARIO === "hang") {
    continue;
  }

  if (q.method === "initialize" && process.env.MCP_SCENARIO === "fail")
    process.exit(1);
  let result;
  if (q.method === "initialize")
    result = {
      protocolVersion: "2025-11-25",
      capabilities: { tools: {}, resources: {} },
      serverInfo: { name: "fixture", version: "1" },
    };
  else if (q.method === "tools/list") {
    if (process.env.MCP_READY_FILE)
      writeFileSync(process.env.MCP_READY_FILE, "ready");
    result = {
      tools: [
        {
          name: "read_fixture",
          description: "Read fixture",
          inputSchema: {
            type: "object",
            properties: { index: { type: "integer" } },
            additionalProperties: false,
          },
          annotations: { readOnlyHint: true },
        },
        {
          name: "write_fixture",
          description: "Write fixture",
          inputSchema: {
            type: "object",
            properties: {},
            additionalProperties: false,
          },
        },
      ],
    };
  } else if (q.method === "tools/call") {
    if (process.env.MCP_RECORD)
      appendFileSync(
        process.env.MCP_RECORD,
        JSON.stringify({ name: q.params.name, pid: process.pid }) + "\n",
      );
    result = { content: [{ type: "text", text: "Fixture worked" }] };
    if (process.env.MCP_SCENARIO === "numeric")
      result = {
        content: [],
        structuredContent: {
          pin: Number(process.env.PIN),
          flag: false,
          none: null,
          safe: "Readable fixture result",
          padding: "N".repeat(25000),
        },
      };
    if (process.env.MCP_SCENARIO === "secrets")
      result = {
        content: [
          {
            type: "text",
            text:
              "Readable result " +
              process.env.API_KEY +
              " " +
              process.env.SHORT_SECRET +
              " " +
              "A".repeat(25000),
          },
          {
            type: "image",
            mimeType: "image/png",
            data: Buffer.from(process.env.API_KEY).toString("base64"),
          },
        ],
      };
  } else if (q.method === "resources/list")
    result = {
      resources: [
        {
          uri: "fixture://read",
          name: "Read fixture resource",
          mimeType: "text/plain",
        },
      ],
    };
  else if (q.method === "resources/read")
    result = {
      contents: [
        {
          uri: "fixture://read",
          mimeType: "text/plain",
          text: "Readable resource",
        },
      ],
    };
  else if (q.method === "resources/templates/list")
    result = { resourceTemplates: [] };
  else {
    send({ id: q.id, error: { code: -32601, message: "Unknown method" } });
    continue;
  }
  if (q.method === "tools/list" && process.env.MCP_SCENARIO === "unsafe_schema")
    result.tools[0].inputSchema.properties.index.default = Number(
      process.env.PIN,
    );
  send({ id: q.id, result });
}
