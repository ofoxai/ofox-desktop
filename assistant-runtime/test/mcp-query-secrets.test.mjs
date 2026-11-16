import test from "node:test";
import assert from "node:assert/strict";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { mcpSecrets, mcpScrubber } from "../src/mcp-security.mjs";
import { writeSessionMcpConfig } from "../src/mcp-config.mjs";
import { sanitizeMcpMessage } from "../src/mcp.mjs";

test("query credentials stay on the transport and leave no SDK or model copy", async (t) => {
  const directory = await mkdtemp(path.join(os.tmpdir(), "ofox-mcp-query-"));
  t.after(() => rm(directory, { recursive: true, force: true }));
  const secret = "synthetic-query !'()~+/汉";
  const component = encodeURIComponent(secret);
  const form = new URLSearchParams({ value: secret }).toString().slice(6);
  const raw = component.replace(/%[0-9A-F]{2}/g, (part) => part.toLowerCase());
  const config = {
    servers: {
      remote: {
        url: `https://example.invalid/mcp?%61pi_key=${raw}&lang=zh&debug=1`,
      },
    },
  };
  const original = structuredClone(config);
  const cleaner = mcpScrubber(mcpSecrets(config));
  for (const value of [secret, raw, component, form])
    assert.ok(
      cleaner.secrets.includes(value),
      "register decoded and encoded credentials",
    );
  for (const value of ["zh", "1"])
    assert.ok(!cleaner.secrets.includes(value), "keep plain query settings");
  const prepared = await writeSessionMcpConfig(directory, config);
  assert.equal(prepared.servers.remote.url, original.servers.remote.url);
  assert.deepEqual(config, original);
  const persisted = await readFile(path.join(directory, "mcp.json"), "utf8");
  assert.match(persisted, /lang=zh&debug=1/);
  const safe = sanitizeMcpMessage(
    {
      jsonrpc: "2.0",
      id: 1,
      result: {
        content: [
          {
            type: "text",
            text: `Readable ${secret} ${raw} ${component} ${form}`,
          },
        ],
      },
    },
    cleaner,
    "tools/call",
  );
  const output = JSON.stringify(safe);
  assert.match(output, /Readable/);
  for (const value of [secret, raw, component, form]) {
    assert.ok(
      !persisted.includes(value),
      "SDK config must not retain credentials",
    );
    assert.ok(
      !output.includes(value),
      "model payload must not retain credentials",
    );
  }
});
