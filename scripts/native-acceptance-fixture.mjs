// A fixed read-only MCP fixture and offline orchestration driver, never an AI proxy.
import { createHash } from "node:crypto";
import { spawn } from "node:child_process";
import { readFile, writeFile } from "node:fs/promises";
import { createInterface } from "node:readline";
import { fileURLToPath } from "node:url";
import { readProfileProof } from "./native-acceptance-drivers.mjs";

const hash = (text) => createHash("sha256").update(text).digest("hex");
const [mode, proofPath, scenario = "success", attempt = "0"] =
  process.argv.slice(2);
const proof = await readProfileProof(proofPath);
const receipt = (input) => ({
  schemaVersion: 1,
  runId: proof.runId,
  sourceSha: proof.sourceSha,
  agentId: proof.agentId,
  nonceHash: hash(proof.nonce),
  instructionHash: hash(input.instructionChallenge),
  skillHash: hash(input.skillChallenge),
  toolName: "attest",
  receiptId: hash(`${proof.runId}:${proof.nonce}:${proof.agentId}`),
});

if (mode === "mcp-server") {
  const lines = createInterface({ input: process.stdin });
  for await (const line of lines) {
    let request;
    try {
      request = JSON.parse(line);
    } catch {
      continue;
    }
    if (request.id === undefined) continue;
    let result;
    if (request.method === "initialize")
      result = {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: "ofox-acceptance-readonly", version: "1" },
      };
    else if (request.method === "tools/list")
      result = {
        tools: [
          {
            name: "attest",
            description:
              "Record the two fixture challenges without side effects outside the isolated acceptance profile.",
            inputSchema: {
              type: "object",
              properties: {
                instructionChallenge: { type: "string" },
                skillChallenge: { type: "string" },
              },
              required: ["instructionChallenge", "skillChallenge"],
              additionalProperties: false,
            },
            annotations: {
              readOnlyHint: true,
              destructiveHint: false,
              idempotentHint: true,
              openWorldHint: false,
            },
          },
        ],
      };
    else if (
      request.method === "tools/call" &&
      request.params?.name === "attest"
    ) {
      const input = request.params.arguments;
      if (
        input?.instructionChallenge !== proof.instructionChallenge ||
        input?.skillChallenge !== proof.skillChallenge
      )
        result = {
          isError: true,
          content: [{ type: "text", text: "challengeMismatch" }],
        };
      else {
        const evidence = receipt(input);
        await writeFile(proof.receipt, `${JSON.stringify(evidence)}\n`, {
          mode: 0o600,
          flag: "wx",
        }).catch(async (error) => {
          if (
            error.code !== "EEXIST" ||
            (await readFile(proof.receipt, "utf8")) !==
              `${JSON.stringify(evidence)}\n`
          )
            throw new Error("receiptConflict");
        });
        result = {
          content: [
            {
              type: "text",
              text: JSON.stringify({
                receiptId: evidence.receiptId,
                accepted: true,
              }),
            },
          ],
        };
      }
    } else {
      process.stdout.write(
        `${JSON.stringify({ jsonrpc: "2.0", id: request.id, error: { code: -32601, message: "Method not found" } })}\n`,
      );
      continue;
    }
    process.stdout.write(
      `${JSON.stringify({ jsonrpc: "2.0", id: request.id, result })}\n`,
    );
  }
} else if (mode === "offline-driver") {
  if (scenario === "timeout")
    await new Promise(() => {
      setInterval(() => {}, 1000);
    });
  if (
    (scenario === "retryOnce" && attempt === "0") ||
    scenario === "failAlways"
  )
    process.exit(7);
  const paths = JSON.parse(await readFile(`${proofPath}.paths`, "utf8"));
  const instructions = await readFile(paths.instructionPath, "utf8");
  const skill = await readFile(paths.skillPath, "utf8");
  const instructionChallenge = instructions.match(
    /instruction challenge is ([a-f0-9]+)/,
  )?.[1];
  const skillChallenge = skill.match(/skill challenge is ([a-f0-9]+)/)?.[1];
  const child = spawn(
    process.execPath,
    [fileURLToPath(import.meta.url), "mcp-server", proofPath],
    { stdio: ["pipe", "pipe", "ignore"], env: process.env, shell: false },
  );
  const replies = createInterface({ input: child.stdout });
  child.stdin.write(
    `${JSON.stringify({ jsonrpc: "2.0", id: 1, method: "initialize", params: { protocolVersion: "2024-11-05", capabilities: {}, clientInfo: { name: "ofox-offline-driver", version: "1" } } })}\n`,
  );
  for await (const line of replies) {
    const reply = JSON.parse(line);
    if (reply.id === 1) {
      child.stdin.write(
        `${JSON.stringify({ jsonrpc: "2.0", method: "notifications/initialized" })}\n`,
      );
      child.stdin.write(
        `${JSON.stringify({ jsonrpc: "2.0", id: 2, method: "tools/call", params: { name: "attest", arguments: { instructionChallenge, skillChallenge } } })}\n`,
      );
    } else if (reply.id === 2) {
      child.stdin.end();
      if (reply.result?.isError) process.exitCode = 8;
      break;
    }
  }
  await new Promise((resolve) => child.on("close", resolve));
  if (scenario === "staleReceipt") {
    const stale = JSON.parse(await readFile(proof.receipt, "utf8"));
    stale.runId = "previous-run";
    await writeFile(proof.receipt, JSON.stringify(stale));
  }
  process.stdout.write(
    `${JSON.stringify({ type: "fixture.completed", agentId: proof.agentId, runId: proof.runId, sourceSha: proof.sourceSha, resourcesRead: 2, mcpCalled: true })}\n`,
  );
} else throw new Error("unsupportedFixtureMode");
