// @vitest-environment node
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { access, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, relative } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  prepareProfile,
  microsoftLearnQuery,
  nativeEvents,
} from "../../scripts/native-acceptance-drivers.mjs";
import {
  verifyAcceptanceIdentity,
  verifyWriterProfile,
} from "../../scripts/native-acceptance-evidence.mjs";
const roots: string[] = [];
const hash = (value: string | Buffer) =>
  createHash("sha256").update(value).digest("hex");
const sha = "a".repeat(40);
async function temp() {
  const path = await mkdtemp(join(tmpdir(), "ofox-evidence-test-"));
  roots.push(path);
  return path;
}
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});
describe("native acceptance identity and resource evidence", () => {
  it("requires both a clean exact-SHA checkout and explicitly pinned matching artifact provenance", async () => {
    const repo = await temp();
    const git = (args: string[]) =>
      execFileSync("git", args, {
        cwd: repo,
        env: {
          PATH: process.env.PATH,
          HOME: repo,
          GIT_CONFIG_NOSYSTEM: "1",
          GIT_CONFIG_GLOBAL: join(repo, "absent-global-config"),
        },
        encoding: "utf8",
        stdio: ["ignore", "pipe", "ignore"],
      }).trim();
    git(["init", "-q"]);
    await writeFile(join(repo, "source.txt"), "tracked source");
    git(["add", "source.txt"]);
    git([
      "-c",
      "user.name=Fixture",
      "-c",
      "user.email=fixture@example.invalid",
      "-c",
      "commit.gpgsign=false",
      "-c",
      "core.hooksPath=/dev/null",
      "commit",
      "-qm",
      "fixture",
    ]);
    const commit = git(["rev-parse", "HEAD"]);
    const artifacts = await temp();
    const path = join(artifacts, "candidate.zip");
    const provenancePath = join(artifacts, "provenance.json");
    await writeFile(path, "pinned candidate");
    const provenance = JSON.stringify({ sourceCommit: commit });
    await writeFile(provenancePath, provenance);
    const manifest = {
      sourceSha: commit,
      artifact: {
        path,
        sha256: hash("pinned candidate"),
        provenancePath,
        provenanceSha256: hash(provenance),
      },
    };
    expect(
      await verifyAcceptanceIdentity(manifest, { checkoutRoot: repo }),
    ).toMatchObject({
      status: "verified",
      sourceSha: commit,
      artifact: { sha256: hash("pinned candidate") },
    });
    await writeFile(join(repo, "source.txt"), "uncommitted");
    expect(
      await verifyAcceptanceIdentity(manifest, { checkoutRoot: repo }),
    ).toMatchObject({
      status: "unverified",
      sourceSha: null,
      checkout: { status: "uncommittedChanges" },
    });
    await writeFile(path, "changed candidate");
    expect(
      await verifyAcceptanceIdentity(manifest, { checkoutRoot: repo }),
    ).toMatchObject({
      sourceSha: null,
      artifact: { status: "mismatchOrUnavailable" },
    });
  });
  it("prepares provider-only writer profiles without manufacturing applied resources", async () => {
    const root = await temp();
    const profile = await prepareProfile({
      root,
      agentId: "codex",
      runId: "prepared-run",
      sourceSha: sha,
      nonce: "a",
      instructionChallenge: "b",
      skillChallenge: "c",
      resourcesMode: "writer",
    });
    expect(profile.skillPath).toBe(
      join(root, "home/.codex/skills/ofox-acceptance/SKILL.md"),
    );
    await expect(access(profile.instructionPath)).rejects.toThrow();
    await expect(access(profile.skillPath)).rejects.toThrow();
    expect(
      await readFile(join(profile.home, ".codex/config.toml"), "utf8"),
    ).not.toContain("mcp_servers");
    const input = JSON.parse(
      await readFile(join(root, "writer-input.json"), "utf8"),
    );
    expect(input.mcp.microsoft_learn.url).toBe(
      "https://learn.microsoft.com/api/mcp",
    );
    expect(input.instructionText).toContain("instruction challenge is b");
  });
  it("checks synthetic writer receipt files, operation results and compiled source identity", async () => {
    const root = await temp();
    const profile = await prepareProfile({
      root,
      agentId: "codex",
      runId: "prepared-run",
      sourceSha: sha,
      nonce: "a",
      instructionChallenge: "b",
      skillChallenge: "c",
    });
    const files = [];
    for (const [kind, path] of [
      ["instructions", profile.instructionPath],
      ["mcp", profile.mcpNativePath],
      ["skills", profile.skillPath],
      ["proof", profile.proofPath],
    ])
      files.push({
        kind,
        relativePath: relative(root, path),
        sha256: hash(await readFile(path)),
      });
    const receipt = {
      schemaVersion: 1,
      writer: "ofoxSandboxWriterV1",
      sourceSha: sha,
      compiledSourceSha: sha,
      compiledSourceClean: true,
      preparedRunId: "prepared-run",
      root,
      agentId: "codex",
      files,
      operations: ["instructions", "mcp", "skills"].map((kind) => ({
        kind,
        operationId: `test-${kind}`,
        result: "applied",
        receiptSha256: "b".repeat(64),
      })),
    };
    const receiptPath = join(root, "writer-receipt.json");
    const raw = JSON.stringify(receipt);
    await writeFile(receiptPath, raw);
    const spec = { root, receiptPath, receiptSha256: hash(raw) };
    expect(
      await verifyWriterProfile(spec, { sourceSha: sha, agentId: "codex" }),
    ).toMatchObject({ writer: "ofoxSandboxWriterV1", verifiedFileCount: 4 });
    // These receipts are deliberately synthetic unit-test inputs. Only the
    // actual Rust sandbox writer supplies receipts for native acceptance.
    for (const compiledIdentity of [
      { compiledSourceSha: "d".repeat(40), compiledSourceClean: true },
      { compiledSourceSha: sha, compiledSourceClean: false },
      { compiledSourceSha: sha, compiledSourceClean: "true" },
      { compiledSourceSha: undefined, compiledSourceClean: undefined },
    ]) {
      const invalidRaw = JSON.stringify({ ...receipt, ...compiledIdentity });
      await writeFile(receiptPath, invalidRaw);
      const result = await verifyWriterProfile(
        { ...spec, receiptSha256: hash(invalidRaw) },
        { sourceSha: sha, agentId: "codex" },
      ).then(
        () => null,
        (error: Error) => error.message,
      );
      expect(result).toBe("writerReceiptIdentityMismatch");
    }
    await writeFile(receiptPath, raw);
    await expect(
      verifyWriterProfile(spec, {
        sourceSha: "c".repeat(40),
        agentId: "codex",
      }),
    ).rejects.toThrow("writerReceiptIdentityMismatch");
    await writeFile(profile.instructionPath, "modified outside writer");
    await expect(
      verifyWriterProfile(spec, { sourceSha: sha, agentId: "codex" }),
    ).rejects.toThrow("writerFileChanged");
  });
  it("requires matching real remote tool-use/result pairs for all five stream protocols", () => {
    const response = "Public documentation result";
    const events = {
      claude: [
        {
          type: "assistant",
          message: {
            content: [
              {
                type: "tool_use",
                id: "r1",
                name: "mcp__microsoft_learn__microsoft_docs_search",
                input: { query: microsoftLearnQuery },
              },
            ],
          },
        },
        {
          type: "user",
          message: {
            content: [
              {
                type: "tool_result",
                tool_use_id: "r1",
                is_error: false,
                content: response,
              },
            ],
          },
        },
      ],
      codex: [
        {
          type: "item.completed",
          item: {
            type: "mcp_tool_call",
            id: "r1",
            server: "microsoft_learn",
            tool: "microsoft_docs_search",
            arguments: { query: microsoftLearnQuery },
            status: "completed",
            result: response,
          },
        },
      ],
      gemini: [
        {
          type: "tool_use",
          tool_name: "microsoft_learn_microsoft_docs_search",
          tool_id: "r1",
          parameters: { query: microsoftLearnQuery },
        },
        {
          type: "tool_result",
          tool_id: "r1",
          status: "success",
          output: response,
        },
      ],
      opencode: [
        {
          type: "tool_use",
          part: {
            tool: "microsoft_learn_microsoft_docs_search",
            callID: "r1",
            state: {
              input: { query: microsoftLearnQuery },
              status: "completed",
              output: response,
            },
          },
        },
      ],
      hermes: [
        {
          type: "tool_use",
          name: "mcp_microsoft_learn_microsoft_docs_search",
          tool_call_id: "r1",
          input: { query: microsoftLearnQuery },
        },
        {
          type: "tool_result",
          name: "mcp_microsoft_learn_microsoft_docs_search",
          tool_call_id: "r1",
          is_error: false,
          output: response,
        },
      ],
    };
    for (const [agent, sample] of Object.entries(events)) {
      const jsonl = sample.map((event) => JSON.stringify(event)).join("\n");
      expect(nativeEvents(agent, jsonl).remoteMcp).toMatchObject({
        status: "verified",
        responseSha256: hash(JSON.stringify(response)),
      });
      expect(
        nativeEvents(
          agent,
          jsonl.replace(microsoftLearnQuery, "different query"),
        ).remoteMcp.status,
      ).toBe("notObserved");
      expect(
        nativeEvents(
          agent,
          JSON.stringify({ type: "text", text: "I searched Microsoft Learn." }),
        ).remoteMcp.status,
      ).toBe("notObserved");
    }
  });
});
