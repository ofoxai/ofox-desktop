// @vitest-environment node
import { mkdtemp, readFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { dirname } from "node:path";
import { rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { afterEach, describe, expect, it } from "vitest";
import {
  BudgetLedger,
  runAcceptance,
  isolatedEnvironment,
} from "../../scripts/native-acceptance.mjs";
import {
  nativeInvocation,
  nativeEvents,
  prepareProfile,
} from "../../scripts/native-acceptance-drivers.mjs";

const roots: string[] = [];
const sourceSha = "a".repeat(40);
interface OfflineArguments {
  _: unknown[];
  prompt?: string;
  "allowed-mcp-server-names"?: string[];
}
interface OfflineParser {
  exitProcess(value: boolean): OfflineParser;
  help(value: boolean): OfflineParser;
  version(value: boolean): OfflineParser;
  option(name: string, value: Record<string, unknown>): OfflineParser;
  fail(callback: (message: string) => never): OfflineParser;
  check(callback: (value: OfflineArguments) => boolean | string): OfflineParser;
  parseSync(): OfflineArguments;
}
async function output() {
  const path = await mkdtemp(join(tmpdir(), "ofox-native-proof-"));
  roots.push(path);
  return path;
}
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((path) => rm(path, { recursive: true, force: true })),
  );
});

describe("native acceptance safety and evidence", () => {
  it("blocks every paid client before spawning when a service-enforced cumulative cap cannot be verified", async () => {
    let spawned = 0;
    const report = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "native",
        sourceSha,
        budgetLimitCents: 1000,
        agents: ["claude", "codex", "gemini", "opencode", "hermes"],
        serverCap: { hardStop: true, limitCents: 1000 },
      },
      {
        outputDirectory: await output(),
        onSpawn: () => {
          spawned += 1;
        },
      },
    );
    expect(spawned).toBe(0);
    expect(report.status).toBe("blocked");
    expect(report.reason).toBe("serverCapVerifierUnavailable");
    expect(report.nativeAccepted).toBe(false);
    expect(report.budget.requestCount).toBe(0);
  });
  it("runs all five isolated fixture drivers and validates MCP server receipts without claiming native usage", async () => {
    const dir = await output();
    const report = await runAcceptance(
      { schemaVersion: 1, mode: "fixture", sourceSha, budgetLimitCents: 1000 },
      { outputDirectory: dir },
    );
    expect(report.status).toBe("fixturePassed");
    expect(report.results).toHaveLength(5);
    expect(
      report.results.every(
        (row: { evidenceLevel: string; status: string }) =>
          row.evidenceLevel === "fixture" && row.status === "passed",
      ),
    ).toBe(true);
    expect(report.nativeAccepted).toBe(false);
    expect(report.budget.settledCents).toBe(0);
    expect(report.sourceSha).toBeNull();
    expect(report.requestedSourceSha).toBe(sourceSha);
    expect(report.environment).toMatchObject({
      platform: process.platform,
      arch: process.arch,
    });
    expect(report.identity.status).toBe("unverified");
    expect(report.billing).toMatchObject({
      currency: "USD",
      actualCents: null,
      status: "notMeasured",
    });
    expect(report.results[0].resources?.instructions.sha256).toMatch(
      /^[a-f0-9]{64}$/,
    );
    expect(report.results[0].clientVersion?.status).toBe("fixtureOnly");
    expect(report.results[0].remoteMcp?.status).toBe("notExecuted");
    expect(JSON.stringify(report)).not.toContain("instructionChallenge");
    expect(await readFile(join(dir, "summary.json"), "utf8")).not.toContain(
      process.env.HOME,
    );
  });
  it.each(["fixture", "writer"] as const)(
    "preseeds the OpenCode schema so native loading preserves %s profile configuration bytes",
    async (resourcesMode) => {
      const profile = await prepareProfile({
        root: await output(),
        agentId: "opencode",
        runId: "offline-schema-regression",
        sourceSha,
        nonce: "offline-nonce",
        instructionChallenge: "offline-instruction",
        skillChallenge: "offline-skill",
        resourcesMode,
      });
      const original = await readFile(profile.mcpNativePath, "utf8");
      const config = JSON.parse(original);
      // OpenCode 1.19.3 inserts this prefix while loading schema-less native
      // configuration. A later byte hash must still match the writer receipt.
      const nativeLoaded = config.$schema
        ? original
        : original.replace(
            /^\s*\{/,
            '{\n  "$schema": "https://opencode.ai/config.json",',
          );
      expect(nativeLoaded).toBe(original);
      expect(config.$schema).toBe("https://opencode.ai/config.json");
    },
  );
  it("retries only once, refuses stale receipts, and cancels without starting later clients", async () => {
    const retry = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "fixture",
        sourceSha,
        agents: ["codex"],
        fixtureScenario: "retryOnce",
      },
      { outputDirectory: await output() },
    );
    expect(retry.results[0].attempts).toBe(2);
    expect(retry.status).toBe("fixturePassed");
    const bad = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "fixture",
        sourceSha,
        agents: ["codex"],
        fixtureScenario: "staleReceipt",
      },
      { outputDirectory: await output() },
    );
    expect(bad.status).toBe("failed");
    expect(bad.results[0].reason).toBe("receiptMismatch");
    const controller = new AbortController();
    const cancelled = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "fixture",
        sourceSha,
        agents: ["codex", "gemini"],
      },
      {
        outputDirectory: await output(),
        signal: controller.signal,
        onSpawn: () => controller.abort(),
      },
    );
    expect(cancelled.status).toBe("cancelled");
    expect(cancelled.results).toHaveLength(1);
  });
  it("holds unknown charges, counts reservations and rejects over-budget calls", () => {
    const budget = new BudgetLedger(1000);
    budget.reserve("one", 700);
    expect(() => budget.reserve("two", 301)).toThrow("localBudgetExceeded");
    budget.settle("one", null);
    expect(budget.snapshot().unresolvedCents).toBe(700);
    expect(() => budget.reserve("two", 301)).toThrow("localBudgetExceeded");
    expect(() => new BudgetLedger(1001)).toThrow("invalidBudgetLimit");
  });
  it("cleans private profiles even when the report directory becomes unwritable", async () => {
    const directory = await output();
    let sandbox = "";
    await expect(
      runAcceptance(
        { schemaVersion: 1, mode: "fixture", sourceSha, agents: ["codex"] },
        {
          outputDirectory: directory,
          onSpawn: ({ cwd }) => {
            sandbox = dirname(dirname(cwd));
            rmSync(directory, { recursive: true, force: true });
            writeFileSync(directory, "report path is now a file");
          },
        },
      ),
    ).rejects.toThrow();
    expect(sandbox).not.toBe("");
    try {
      const privateProfileSurvived = await readFile(
        join(sandbox, "codex-0/fixture-private.json"),
      ).then(
        () => true,
        () => false,
      );
      expect(privateProfileSurvived).toBe(false);
    } finally {
      await rm(sandbox, { recursive: true, force: true });
    }
  });
  it("terminates timed-out fixture process trees, limits retries, and releases only proven zero-cost reservations", async () => {
    const report = await runAcceptance(
      {
        schemaVersion: 1,
        mode: "fixture",
        sourceSha,
        agents: ["hermes"],
        fixtureScenario: "timeout",
        timeoutMs: 100,
      },
      { outputDirectory: await output() },
    );
    expect(report.status).toBe("failed");
    expect(report.results[0]).toMatchObject({ attempts: 2, reason: "timeout" });
    expect(report.budget.unresolvedCents).toBe(0);
    expect(report.budget.requestCount).toBe(2);
    await expect(
      runAcceptance(
        { schemaVersion: 1, mode: "fixture", sourceSha, shell: "echo unsafe" },
        { outputDirectory: await output() },
      ),
    ).rejects.toThrow("unsafeManifestField");
    await expect(
      runAcceptance(
        { schemaVersion: 1, mode: "fixture", sourceSha, maxRetries: 2 },
        { outputDirectory: await output() },
      ),
    ).rejects.toThrow("invalidRetryLimit");
  });
  it("removes inherited credentials, proxies and config overrides from every child", () => {
    const env = isolatedEnvironment("/sandbox", {
      PATH: "/tools",
      OPENAI_API_KEY: "live-secret",
      HTTP_PROXY: "https://secret",
      CODEX_HOME: "/production",
      SSH_AUTH_SOCK: "/ssh",
      NODE_OPTIONS: "--require=evil",
    });
    expect(env.CODEX_HOME).toBe("/sandbox/home/.codex");
    expect(env.OPENAI_API_KEY).toBeUndefined();
    expect(env.HTTP_PROXY).toBeUndefined();
    expect(env.SSH_AUTH_SOCK).toBeUndefined();
    expect(env.NODE_OPTIONS).toBeUndefined();
  });
  it("builds fixed structured native client invocations without shell or bypass flags", () => {
    const expected = {
      claude: "stream-json",
      codex: "--json",
      gemini: "stream-json",
      opencode: "json",
      hermes: "stream-json",
    };
    for (const [agentId, flag] of Object.entries(expected)) {
      const invocation = nativeInvocation(agentId, {
        workspace: "/sandbox/work",
        prompt: "fixed acceptance",
        model: "test-model",
      });
      expect(invocation.args).toContain(flag);
      expect(invocation.args.join(" ")).not.toMatch(
        /yolo|dangerously|bypass-permissions|--auto\b/,
      );
      expect(invocation.shell).toBe(false);
      if (agentId === "claude") {
        expect(invocation.args).toContain("--bare");
        expect(invocation.args).toContain("--append-system-prompt-file");
        expect(invocation.args.at(-1)).toBe(
          "/ofox-acceptance\nfixed acceptance",
        );
        expect(invocation.args.join(" ")).not.toContain("SKILL.md");
      } else {
        expect(invocation.args).toContain("fixed acceptance");
        expect(invocation.args.join(" ")).not.toContain("/ofox-acceptance");
      }
    }
  });
  it("passes both Gemini MCP names through its nargs-one parser without creating a positional prompt", () => {
    // Gemini 0.64.0 bundles yargs with this exact option and comma coercion.
    // Reuse the lockfile's existing MSW yargs dependency for an offline replay;
    // no Gemini entry point, config, credential or network is accessed here.
    const installedDependency = createRequire(
      createRequire(import.meta.url).resolve("msw/package.json"),
    );
    const yargs = installedDependency("yargs/yargs") as (
      args: string[],
    ) => OfflineParser;
    const invocation = nativeInvocation("gemini", {
      workspace: "/sandbox/work",
      prompt: "fixed acceptance",
      model: "test-model",
    });
    const parsed = yargs(invocation.args)
      .exitProcess(false)
      .help(false)
      .version(false)
      .option("prompt", { type: "string" })
      .option("allowed-mcp-server-names", {
        type: "array",
        string: true,
        nargs: 1,
        coerce: (values: string[]) =>
          values.flatMap((value) =>
            value
              .split(",")
              .map((item) => item.trim())
              .filter(Boolean),
          ),
      })
      .check((value) =>
        value.prompt && value._.length > 0
          ? "Cannot use both a positional prompt and the --prompt (-p) flag together"
          : true,
      )
      .fail((message) => {
        throw new Error(message);
      })
      .parseSync();
    expect(parsed["allowed-mcp-server-names"]).toEqual([
      "ofox_acceptance",
      "microsoft_learn",
    ]);
    expect(parsed._).toEqual([]);
    expect(parsed.prompt).toBe("fixed acceptance");
  });
  it("accepts structured real tool events but never assistant self-reports or incomplete runs", () => {
    const samples = {
      claude: [
        {
          type: "assistant",
          message: {
            content: [
              {
                type: "tool_use",
                name: "mcp__ofox_acceptance__attest",
                id: "c1",
              },
            ],
          },
        },
        { type: "result", subtype: "success", is_error: false },
      ],
      codex: [
        {
          type: "item.completed",
          item: {
            type: "mcp_tool_call",
            server: "ofox_acceptance",
            tool: "attest",
            status: "completed",
            id: "c1",
          },
        },
        { type: "turn.completed", usage: { input_tokens: 1 } },
      ],
      gemini: [
        {
          type: "tool_use",
          tool_name: "ofox_acceptance_attest",
          tool_id: "c1",
        },
        { type: "result", status: "success" },
      ],
      opencode: [
        {
          type: "tool_use",
          part: {
            tool: "ofox_acceptance_attest",
            state: { status: "completed" },
            callID: "c1",
          },
        },
        { type: "step_finish", part: { reason: "stop" } },
      ],
      hermes: [
        {
          type: "tool_use",
          name: "ofox_acceptance_attest",
          tool_call_id: "c1",
        },
        { type: "result", exit_code: 0 },
      ],
    };
    for (const [agent, events] of Object.entries(samples)) {
      expect(
        nativeEvents(
          agent,
          events.map((event) => JSON.stringify(event)).join("\n"),
        ),
      ).toMatchObject({ completed: true, mcpCalled: true });
      expect(
        nativeEvents(
          agent,
          JSON.stringify({
            type: "text",
            text: "I loaded the skill and called the MCP.",
          }),
        ),
      ).toMatchObject({ completed: false, mcpCalled: false });
    }
  });
});
