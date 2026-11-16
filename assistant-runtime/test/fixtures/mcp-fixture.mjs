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
            properties: {
              target: { type: "string" },
              body: { type: "string" },
              nested: { type: "object" },
            },
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
  if (
    q.method === "tools/list" &&
    process.env.MCP_SCENARIO === "schema_classification"
  ) {
    const schema = result.tools[0].inputSchema;
    const pin = Number(process.env.PIN);
    const cases = {
      extension_value: { "x-credential": pin },
      extension_nested: { "x-private": { value: pin } },
      disguised_schema: {
        "x-private": { type: "object", properties: { flag: pin } },
      },
      disguised_annotation: {
        "x-private": { annotations: { readOnlyHint: pin } },
      },
      disguised_root: {
        "x-private": { inputSchema: { properties: { flag: pin } } },
      },
      safe_metadata: {
        "x-private": {
          type: "display",
          properties: { label: "Readable metadata" },
          annotations: { readOnlyHint: "display only" },
        },
      },
    };
    if (process.env.SCHEMA_CASE === "legitimate") {
      schema.properties.safe = false;
      schema.properties.list = {
        type: "array",
        items: false,
        prefixItems: [true, false],
        contains: true,
        unevaluatedItems: false,
      };
      schema.properties.choice = {
        allOf: [true, { anyOf: [false, true] }],
        oneOf: [false, true],
        not: false,
        if: true,
        then: true,
        else: false,
      };
      schema.additionalProperties = false;
      schema.unevaluatedProperties = false;
      schema.propertyNames = true;
      schema.$defs = { never: false };
      schema.properties.legacy = {
        type: "array",
        items: [true, false],
        additionalItems: false,
      };
      result.tools[0].annotations = {
        readOnlyHint: true,
        destructiveHint: false,
      };
    } else if (process.env.SCHEMA_CASE === "root_bool")
      result.tools[0].inputSchema = false;
    else Object.assign(schema, cases[process.env.SCHEMA_CASE]);
  }
  if (process.env.MCP_SCENARIO === "metadata_boundary") {
    const metadata = {
      annotations: {
        readOnlyHint: false,
        audience: ["assistant"],
        priority: 0.5,
      },
      vendor: {
        inputSchema: { type: "object", additionalProperties: false },
        outputSchema: { type: "object", properties: { safe: false } },
        annotations: { readOnlyHint: false },
        nested: {
          isError: false,
          type: "text",
          tools: [{ annotations: { readOnlyHint: false } }],
          resources: [{ inputSchema: { type: "object" } }],
        },
        data: { pin: 418731, empty: null, 418731: "Readable key" },
        readable: "Readable resource metadata",
      },
      description: "Readable resource metadata",
      padding: "R".repeat(25000),
    };
    if (q.method === "resources/list")
      Object.assign(result.resources[0], metadata);
    if (q.method === "resources/templates/list")
      result.resourceTemplates = [
        {
          uriTemplate: "fixture://read/{index}",
          name: "Readable template",
          ...metadata,
        },
      ];
    if (q.method === "resources/read") {
      Object.assign(result.contents[0], metadata);
      result.contents[0].text = JSON.stringify({
        readable: "Readable resource body",
        pin: 418731,
        flag: false,
        empty: null,
        padding: "R".repeat(25000),
      });
    }
    if (q.method === "tools/list") {
      result.tools[0].outputSchema = {
        type: "object",
        properties: { safe: false },
        additionalProperties: false,
      };
      result.tools[0].annotations = {
        readOnlyHint: true,
        destructiveHint: false,
      };
    }
  }
  send({ id: q.id, result });
}
