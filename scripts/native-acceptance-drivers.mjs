import {
  resourceClientIds,
  resourceTargetId,
  acceptanceResourceKinds,
} from "./resource-targets.mjs";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { createHash } from "node:crypto";

export const agentIds = Object.freeze([
  "claude",
  "codex",
  "gemini",
  "opencode",
  "hermes",
]);
const fixtureServer = fileURLToPath(
  new URL("./native-acceptance-fixture.mjs", import.meta.url),
);
const quote = (value) => JSON.stringify(value);
export const microsoftLearnEndpoint = "https://learn.microsoft.com/api/mcp";
export const microsoftLearnQuery =
  "What is Microsoft Learn MCP Server and how can developers search public documentation?";
export function profilePaths(root, agentId) {
  if (!resourceClientIds.includes(agentId)) throw new Error("unsupportedAgent");
  const home = join(root, "home");
  const target = resourceTargetId(agentId);
  const instructions = {
    claude: ".claude/CLAUDE.md",
    codex: ".codex/AGENTS.md",
    gemini: ".gemini/GEMINI.md",
    opencode: ".config/opencode/AGENTS.md",
    hermes: ".hermes/AGENTS.md",
  };
  const skills = {
    claude: ".claude/skills/ofox-acceptance/SKILL.md",
    codex: ".codex/skills/ofox-acceptance/SKILL.md",
    gemini: ".gemini/skills/ofox-acceptance/SKILL.md",
    opencode: ".config/opencode/skills/ofox-acceptance/SKILL.md",
    hermes: ".hermes/skills/ofox-acceptance/SKILL.md",
    openclaw: ".openclaw/skills/ofox-acceptance/SKILL.md",
    workbuddy: ".workbuddy/skills/ofox-acceptance/SKILL.md",
    dsh: ".dsh/skills/ofox-acceptance/SKILL.md",
    pi: ".pi/agent/skills/ofox-acceptance/SKILL.md",
  };
  const mcpPaths = {
    claude: ".claude/.claude.json",
    codex: ".codex/config.toml",
    gemini: ".gemini/settings.json",
    opencode: ".config/opencode/opencode.json",
    hermes: ".hermes/config.yaml",
    openclaw: ".openclaw/openclaw.json",
    workbuddy: ".workbuddy/mcp.json",
    dsh: ".dsh/cordis.patch.yml",
    pi: ".pi/agent/mcp.json",
  };
  return {
    root,
    home,
    workspace: join(root, "work"),
    receipt: join(root, "mcp-receipt.json"),
    proofPath: join(root, "fixture-private.json"),
    instructionPath: join(
      home,
      instructions[target] ?? ".acceptance-unused-instructions",
    ),
    skillPath: join(home, skills[target]),
    mcpConfig: join(root, "mcp.json"),
    mcpNativePath: join(home, mcpPaths[target]),
  };
}
export function acceptancePrompt(resourceKinds) {
  if (!acceptanceResourceKinds(resourceKinds).includes("instructions"))
    return "Load the ofox-acceptance skill through your native Skills mechanism. Invoke the local ofox_acceptance attest MCP tool exactly once with the two challenge values from that skill. Do not print the challenges, execute shell, modify files, use other tools or call other services. Finish after the receipt.";
  return `Perform the read-only OFox acceptance task described in your native instructions. Load the ofox-acceptance skill and invoke the local ofox_acceptance attest MCP tool once with the two challenge values. Then use the microsoft_learn MCP server's microsoft_docs_search tool once with query: ${microsoftLearnQuery} Do not print the challenges, run shell commands, modify files, use other tools or call other services. Finish after both tool results.`;
}
async function file(path, value) {
  await mkdir(dirname(path), { recursive: true, mode: 0o700 });
  await writeFile(path, value, { mode: 0o600 });
}

// These are fixed adapters, based on installed --help / native stream protocols.
// No manifest may supply a shell, CLI arguments, user prompt, plugin or executable code.
export function nativeInvocation(
  agentId,
  { workspace, prompt, model, mcpConfig, home, instructionPath, resourceKinds },
) {
  if (!resourceClientIds.includes(agentId)) throw new Error("unsupportedAgent");
  if (["chatgpt", "workbuddy"].includes(agentId))
    throw new Error("desktopEntryRequired");
  const withInstructions =
    acceptanceResourceKinds(resourceKinds).includes("instructions");
  if (!model || !/^[\w./:+-]{1,160}$/.test(model))
    throw new Error("invalidTestModel");
  const args = {
    claude: [
      "--bare",
      "--print",
      "--output-format",
      "stream-json",
      "--verbose",
      "--restricted",
      "--tools",
      "Read,Skill",
      "--allowed-tools",
      "Read,Skill,mcp__ofox_acceptance__attest,mcp__microsoft_learn__microsoft_docs_search",
      "--permission-prompts",
      "none",
      "--strict-mcp-config",
      "--mcp-config",
      mcpConfig ?? join(workspace, "mcp.json"),
      "--settings",
      join(home ?? workspace, ".claude/settings.json"),
      "--append-system-prompt-file",
      instructionPath ?? join(home ?? workspace, ".claude/CLAUDE.md"),
      "--model",
      model,
      `/ofox-acceptance\n${prompt}`,
    ],
    codex: [
      "exec",
      "--json",
      "--strict-config",
      "--sandbox",
      "read-only",
      "--skip-git-repo-check",
      "--ephemeral",
      "--cd",
      workspace,
      "--model",
      model,
      prompt,
    ],
    gemini: [
      "--prompt",
      prompt,
      "--output-format",
      "stream-json",
      "--approval-mode",
      "plan",
      "--allowed-mcp-server-names",
      "ofox_acceptance,microsoft_learn",
      "--model",
      model,
    ],
    opencode: [
      "run",
      "--pure",
      "--format",
      "json",
      "--dir",
      workspace,
      "--model",
      `ofox-acceptance/${model}`,
      prompt,
    ],
    pi: [
      "--mode",
      "json",
      "--print",
      "--no-session",
      "--no-context-files",
      "--no-extensions",
      "-e",
      "builtin:mcp",
      "--no-prompt-templates",
      "--no-themes",
      "--tools",
      "read,mcp__ofox_acceptance__attest",
      "--provider",
      "ofox-acceptance",
      "--model",
      model,
      "--thinking",
      "off",
      prompt,
    ],
    dsh: ["headless", "--json", prompt],
    openclaw: [
      "agent",
      "exec",
      "--config",
      join(home ?? workspace, ".openclaw/openclaw.json"),
      "--state-dir",
      join(home ?? workspace, ".openclaw"),
      "--cwd",
      workspace,
      "--model",
      `ofox-acceptance/${model}`,
      "--thinking",
      "off",
      "--timeout",
      "90",
      "--json",
      prompt,
    ],
    hermes: [
      "chat",
      "--query",
      prompt,
      "--oneshot",
      "--format",
      "stream-json",
      "--toolsets",
      "skills,mcp",
      "--skills",
      "ofox-acceptance",
      "--provider",
      "ofox-acceptance",
      "--model",
      model,
    ],
  }[agentId];
  if (!withInstructions && agentId === "claude") {
    args.splice(args.indexOf("--append-system-prompt-file"), 2);
    args[args.indexOf("--allowed-tools") + 1] =
      "Read,Skill,mcp__ofox_acceptance__attest";
  }
  if (!withInstructions && agentId === "gemini")
    args[args.indexOf("--allowed-mcp-server-names") + 1] = "ofox_acceptance";
  return { args, cwd: workspace, shell: false };
}

export function nativeEvents(agentId, output) {
  if (!resourceClientIds.includes(agentId)) throw new Error("unsupportedAgent");
  if (["chatgpt", "workbuddy"].includes(agentId))
    throw new Error("desktopEntryRequired");
  if (agentId === "openclaw") {
    let envelope;
    try {
      envelope = JSON.parse(output.trim());
    } catch {
      envelope = {};
    }
    const names = Array.isArray(envelope.toolSummary?.tools)
      ? envelope.toolSummary.tools
      : [];
    const calls = names.filter((name) =>
      /^(?:mcp__|mcp_)?ofox_acceptance(?:__|_)attest$/.test(name),
    );
    return {
      completed:
        envelope.ok === true && envelope.status === "ok" && !envelope.error,
      mcpCalled: calls.length > 0 && envelope.toolSummary.failures === 0,
      structuredEvents: envelope.status ? 1 : 0,
      toolCallCount: calls.length,
      remoteMcp: {
        status: "notObserved",
        endpoint: microsoftLearnEndpoint,
        tool: "microsoft_docs_search",
        reason: "nativeToolResultNotObserved",
      },
    };
  }
  let completed = false;
  let mcpCalled = false;
  let failed = false;
  let structuredEvents = 0;
  const calls = new Set();
  const remoteCalls = new Map();
  let remoteMcp = {
    status: "notObserved",
    endpoint: microsoftLearnEndpoint,
    tool: "microsoft_docs_search",
    reason: "nativeToolResultNotObserved",
  };
  const remoteName = (name) =>
    /^(?:mcp__|mcp_)?microsoft_learn(?:__|_)microsoft_docs_search$/.test(
      name ?? "",
    );
  const remoteRequest = (id, input) => {
    if (typeof input === "string") {
      try {
        input = JSON.parse(input);
      } catch {
        return;
      }
    }
    if (typeof id === "string" && input?.query === microsoftLearnQuery)
      remoteCalls.set(id, true);
  };
  const remoteResult = (id, result, success) => {
    if (
      !remoteCalls.has(id) ||
      !success ||
      result === undefined ||
      result === null ||
      result === ""
    )
      return;
    remoteMcp = {
      status: "verified",
      endpoint: microsoftLearnEndpoint,
      tool: "microsoft_docs_search",
      evidence: "nativeClientToolResult",
      callIdHash: createHash("sha256").update(id).digest("hex"),
      responseSha256: createHash("sha256")
        .update(JSON.stringify(result))
        .digest("hex"),
    };
  };
  for (const line of output.split(/\r?\n/)) {
    let event;
    try {
      event = JSON.parse(line);
    } catch {
      continue;
    }
    if (!event || typeof event !== "object" || typeof event.type !== "string")
      continue;
    structuredEvents += 1;
    if (agentId === "claude") {
      for (const item of Array.isArray(event.message?.content)
        ? event.message.content
        : []) {
        if (
          item.type === "tool_use" &&
          item.name === "mcp__ofox_acceptance__attest"
        ) {
          mcpCalled = true;
          calls.add(item.id);
        }
        if (item.type === "tool_use" && remoteName(item.name))
          remoteRequest(item.id, item.input);
        if (event.type === "user" && item.type === "tool_result")
          remoteResult(item.tool_use_id, item.content, item.is_error !== true);
      }
      if (event.type === "result") {
        completed = event.subtype === "success" && event.is_error === false;
        failed ||= !completed;
      }
    } else if (agentId === "codex") {
      if (
        event.type === "item.completed" &&
        event.item?.type === "mcp_tool_call" &&
        event.item.server === "microsoft_learn" &&
        event.item.tool === "microsoft_docs_search"
      ) {
        remoteRequest(event.item.id, event.item.arguments);
        remoteResult(
          event.item.id,
          event.item.result,
          event.item.status === "completed" && !event.item.error,
        );
      }
      if (
        event.type === "item.completed" &&
        event.item?.type === "mcp_tool_call" &&
        event.item.server === "ofox_acceptance" &&
        event.item.tool === "attest" &&
        event.item.status === "completed"
      ) {
        mcpCalled = true;
        calls.add(event.item.id);
      }
      completed ||= event.type === "turn.completed";
      failed ||= ["turn.failed", "error"].includes(event.type);
    } else if (agentId === "gemini") {
      if (event.type === "tool_use" && remoteName(event.tool_name))
        remoteRequest(event.tool_id, event.parameters);
      if (event.type === "tool_result")
        remoteResult(event.tool_id, event.output, event.status === "success");
      if (
        event.type === "tool_use" &&
        /^(?:mcp__)?ofox_acceptance(?:__|_)attest$/.test(event.tool_name ?? "")
      ) {
        mcpCalled = true;
        calls.add(event.tool_id);
      }
      if (event.type === "result") {
        completed = event.status === "success";
        failed ||= !completed;
      }
    } else if (agentId === "opencode") {
      if (event.type === "tool_use" && remoteName(event.part?.tool)) {
        remoteRequest(event.part.callID, event.part.state?.input);
        remoteResult(
          event.part.callID,
          event.part.state?.output,
          event.part.state?.status === "completed",
        );
      }
      if (
        event.type === "tool_use" &&
        event.part?.tool === "ofox_acceptance_attest" &&
        event.part.state?.status === "completed"
      ) {
        mcpCalled = true;
        calls.add(event.part.callID);
      }
      completed ||=
        event.type === "step_finish" && event.part?.reason === "stop";
      failed ||= event.type === "error";
    } else if (agentId === "pi") {
      if (
        event.type === "tool_execution_end" &&
        event.toolName === "mcp__ofox_acceptance__attest" &&
        event.isError === false
      ) {
        mcpCalled = true;
        calls.add(event.toolCallId);
      }
      if (event.type === "agent_end") {
        completed = true;
        failed ||= (event.messages ?? []).some((message) =>
          ["error", "aborted"].includes(message.stopReason),
        );
      }
    } else if (agentId === "dsh") {
      if (
        event.type === "tool_call" &&
        /^(?:mcp__|mcp_)?ofox_acceptance(?:__|_)attest$/.test(event.tool)
      )
        calls.add(event.callId);
      if (
        event.type === "tool_result" &&
        calls.has(event.callId) &&
        event.status === "completed"
      )
        mcpCalled = true;
      completed ||= event.type === "final";
      failed ||= event.type === "error";
    } else {
      if (event.type === "tool_use" && remoteName(event.name))
        remoteRequest(event.tool_call_id, event.input);
      if (event.type === "tool_result")
        remoteResult(
          event.tool_call_id,
          event.output,
          event.is_error === false,
        );
      if (
        event.type === "tool_use" &&
        /^(?:mcp_)?ofox_acceptance(?:__|_)attest$/.test(event.name ?? "")
      ) {
        mcpCalled = true;
        calls.add(event.tool_call_id);
      }
      if (event.type === "result") {
        completed = event.exit_code === 0;
        failed ||= !completed;
      }
    }
  }
  return {
    completed: completed && !failed,
    mcpCalled,
    structuredEvents,
    toolCallCount: calls.size,
    remoteMcp,
  };
}

export async function prepareProfile({
  root,
  agentId,
  runId,
  sourceSha,
  nonce,
  instructionChallenge,
  skillChallenge,
  model = "fixture-model",
  apex = "ofox.ai",
  credential,
  resourcesMode = "fixture",
  resourceKinds: requestedKinds,
}) {
  const resourceKinds = acceptanceResourceKinds(requestedKinds);
  if (
    !resourceClientIds.includes(agentId) ||
    !["ofox.ai", "ofox.io"].includes(apex)
  )
    throw new Error("invalidProfile");
  const targetId = resourceTargetId(agentId);
  const paths = profilePaths(root, agentId);
  if (resourceKinds.includes("instructions") && !agentIds.includes(targetId))
    throw new Error("instructionTargetUnsupported");
  const home = join(root, "home");
  const workspace = join(root, "work");
  const receipt = join(root, "mcp-receipt.json");
  const proofPath = join(root, "fixture-private.json");
  const instructionPart = resourceKinds.includes("instructions")
    ? ""
    : `The instruction challenge is ${instructionChallenge}.\n`;
  const skill = `---\nname: ofox-acceptance\ndescription: Read-only OFox acceptance challenge; never run a script.\n---\nThe skill challenge is ${skillChallenge}. Supply it only to the local ofox_acceptance attest tool.\n${instructionPart}`;
  const instructions = `# OFox acceptance\nThe instruction challenge is ${instructionChallenge}. Use the ofox-acceptance skill, then call the local ofox_acceptance attest tool with both challenges. Never execute shell, install dependencies or modify files.\n`;
  const directories = [
    home,
    workspace,
    ".claude",
    ".codex",
    ".gemini",
    ".agents/skills/ofox-acceptance",
    ".config/opencode",
    ".hermes",
  ].map((path) =>
    path === home || path === workspace ? path : join(home, path),
  );
  for (const path of directories)
    await mkdir(path, { recursive: true, mode: 0o700 });
  const mcp = {
    command: process.execPath,
    args: [fixtureServer, "mcp-server", proofPath],
  };
  await file(
    proofPath,
    JSON.stringify({
      schemaVersion: 1,
      runId,
      sourceSha,
      agentId,
      resourceKinds,
      nonce,
      instructionChallenge,
      skillChallenge,
      receipt,
    }),
  );
  if (!["fixture", "writer"].includes(resourcesMode))
    throw new Error("invalidResourcesMode");
  if (resourcesMode === "fixture" && resourceKinds.includes("instructions"))
    await file(paths.instructionPath, instructions);
  const skillPath = paths.skillPath;
  if (resourcesMode === "fixture") await file(skillPath, skill);
  const mcpConfig = join(root, "mcp.json");
  await file(
    mcpConfig,
    JSON.stringify({
      mcpServers:
        resourcesMode === "writer"
          ? {}
          : {
              ofox_acceptance: mcp,
              microsoft_learn: { type: "http", url: microsoftLearnEndpoint },
            },
    }),
  );
  const gateway = `https://api.${apex}`;
  const key = credential ?? "fixture-not-a-live-key";
  if (agentId === "claude") {
    await file(
      join(home, ".claude/.claude.json"),
      JSON.stringify({
        hasCompletedOnboarding: true,
        mcpServers:
          resourcesMode === "writer"
            ? {}
            : {
                ofox_acceptance: mcp,
                microsoft_learn: { type: "http", url: microsoftLearnEndpoint },
              },
      }),
    );
    await file(
      join(home, ".claude/settings.json"),
      JSON.stringify({
        env: {
          ANTHROPIC_BASE_URL: `${gateway}/anthropic`,
          ...(resourcesMode === "writer" ? {} : { ANTHROPIC_API_KEY: key }),
          DISABLE_AUTOUPDATER: "1",
        },
        permissions: {
          allow: [
            "Read",
            "Skill",
            "mcp__ofox_acceptance__attest",
            "mcp__microsoft_learn__microsoft_docs_search",
          ],
          deny: ["Bash", "Write", "Edit", "WebFetch", "WebSearch"],
        },
      }),
    );
  } else if (targetId === "codex") {
    await file(
      join(home, ".codex/config.toml"),
      `model_provider = "ofox-acceptance"\nmodel = ${quote(model)}\napproval_policy = "never"\nsandbox_mode = "read-only"\n[model_providers.ofox-acceptance]\nname = "OFox acceptance"\nbase_url = ${quote(`${gateway}/v1`)}\nwire_api = "responses"\nenv_key = "OFOX_ACCEPTANCE_TEST_KEY"\n` +
        (resourcesMode === "writer"
          ? ""
          : `[mcp_servers.ofox_acceptance]\ncommand = ${quote(mcp.command)}\nargs = ${quote(mcp.args)}\n[mcp_servers.microsoft_learn]\nurl = ${quote(microsoftLearnEndpoint)}\n`),
    );
  } else if (agentId === "gemini") {
    await file(
      join(home, ".gemini/settings.json"),
      JSON.stringify({
        security: { auth: { selectedType: "gemini-api-key" } },
        mcpServers:
          resourcesMode === "writer"
            ? {}
            : {
                ofox_acceptance: { ...mcp, trust: true },
                microsoft_learn: { httpUrl: microsoftLearnEndpoint },
              },
        general: { disableAutoUpdate: true },
      }),
    );
  } else if (agentId === "opencode") {
    await file(
      join(home, ".config/opencode/opencode.json"),
      JSON.stringify({
        $schema: "https://opencode.ai/config.json",
        model: `ofox-acceptance/${model}`,
        provider: {
          "ofox-acceptance": {
            npm: "@ai-sdk/openai-compatible",
            name: "OFox acceptance",
            options: {
              baseURL: `${gateway}/v1`,
              apiKey:
                resourcesMode === "writer"
                  ? "{env:OFOX_ACCEPTANCE_TEST_KEY}"
                  : key,
            },
            models: { [model]: { name: model } },
          },
        },
        permission: {
          "*": "deny",
          read: "allow",
          skill: "allow",
          "ofox_acceptance_*": "allow",
          microsoft_learn_microsoft_docs_search: "allow",
        },
        mcp:
          resourcesMode === "writer"
            ? {}
            : {
                ofox_acceptance: {
                  type: "local",
                  command: [mcp.command, ...mcp.args],
                  enabled: true,
                },
                microsoft_learn: {
                  type: "remote",
                  url: microsoftLearnEndpoint,
                  enabled: true,
                },
              },
      }),
    );
  } else if (targetId === "hermes") {
    // A native block mapping lets the real writer preserve unrelated YAML fields.
    const hermesConfig = {
      _config_version: 50,
      model: { provider: "ofox-acceptance", default: model },
      providers: {
        "ofox-acceptance": {
          name: "OFox acceptance",
          api: `${gateway}/v1`,
          ...(resourcesMode === "writer" ? {} : { api_key: key }),
          transport: "chat_completions",
          models: { [model]: {} },
          default_model: model,
        },
      },
      agent: { max_turns: 6 },
      skills: { enabled: true },
      toolsets: ["skills", "mcp"],
      mcp_servers:
        resourcesMode === "writer"
          ? {}
          : {
              ofox_acceptance: mcp,
              microsoft_learn: { url: microsoftLearnEndpoint },
            },
    };
    await file(
      join(home, ".hermes/config.yaml"),
      resourcesMode === "writer"
        ? Object.entries(hermesConfig)
            .map(([key, value]) => `${key}: ${JSON.stringify(value)}`)
            .join("\n") + "\n"
        : JSON.stringify(hermesConfig),
    );
  }
  if (["pi", "openclaw", "dsh", "workbuddy"].includes(targetId)) {
    if (resourcesMode !== "writer")
      throw new Error("newTargetsRequireRealWriter");
    const modelRow = {
      id: model,
      name: model,
      reasoning: false,
      input: ["text"],
      contextWindow: 32768,
      maxTokens: 1024,
    };
    if (targetId === "pi") {
      await file(
        join(home, ".pi/agent/models.json"),
        JSON.stringify({
          providers: {
            "ofox-acceptance": {
              baseUrl: `${gateway}/v1`,
              api: "openai-completions",
              apiKey: "OFOX_ACCEPTANCE_TEST_KEY",
              models: [modelRow],
            },
          },
        }),
      );
      await file(
        join(home, ".pi/agent/settings.json"),
        JSON.stringify({
          defaultProvider: "ofox-acceptance",
          defaultModel: model,
          telemetry: false,
        }),
      );
    }
    if (targetId === "dsh") {
      const patches = [
        {
          id: "llm-pi-ai",
          config: {
            providers: {
              "ofox-acceptance": {
                displayName: "Acceptance",
                api: "openai-completions",
                baseURL: `${gateway}/v1`,
                apiKeyEnv: "OFOX_ACCEPTANCE_TEST_KEY",
                models: [modelRow],
              },
            },
          },
        },
        {
          id: "agent-default-model",
          config: { provider: "ofox-acceptance", model },
        },
      ];
      await file(paths.mcpNativePath, JSON.stringify(patches));
    } else {
      await file(
        paths.mcpNativePath,
        JSON.stringify(
          targetId === "openclaw"
            ? {
                models: {
                  providers: {
                    "ofox-acceptance": {
                      baseUrl: `${gateway}/v1`,
                      api: "openai-completions",
                      apiKey: {
                        source: "env",
                        provider: "default",
                        id: "OFOX_ACCEPTANCE_TEST_KEY",
                      },
                      models: [modelRow],
                    },
                  },
                },
                agents: { defaults: { workspace } },
                tools: {
                  allow: ["read", "mcp__ofox_acceptance__attest"],
                  deny: [
                    "exec",
                    "write",
                    "edit",
                    "apply_patch",
                    "browser",
                    "web_fetch",
                    "web_search",
                  ],
                },
                mcp: { servers: {} },
              }
            : { mcpServers: {} },
        ),
      );
    }
  }
  await file(
    join(root, "writer-input.json"),
    JSON.stringify({
      schemaVersion: 1,
      sourceSha,
      agentId,
      preparedRunId: runId,
      resourceKinds,
      instructionText: instructions,
      skillText: skill,
      mcp: {
        ofox_acceptance:
          targetId === "pi" ? { ...mcp, exposure: "direct" } : mcp,
        ...(resourceKinds.includes("instructions")
          ? { microsoft_learn: { type: "http", url: microsoftLearnEndpoint } }
          : {}),
      },
    }),
  );
  await file(
    join(root, ".ofox-acceptance-prepared.json"),
    JSON.stringify({
      schemaVersion: 1,
      sourceSha,
      agentId,
      preparedRunId: runId,
      resourceKinds,
      expiresAt: Date.now() + 60 * 60_000,
      privateProofSha256: createHash("sha256")
        .update(await readFile(proofPath))
        .digest("hex"),
      resourcesMode,
    }),
  );
  const prompt = acceptancePrompt(resourceKinds);
  return {
    ...profilePaths(root, agentId),
    root,
    home,
    workspace,
    receipt,
    proofPath,
    instructionPath: paths.instructionPath,
    skillPath,
    mcpConfig,
    prompt,
    resourceKinds,
    model,
    key,
  };
}

export async function readProfileProof(path) {
  return JSON.parse(await readFile(path, "utf8"));
}
