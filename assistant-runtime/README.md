# Bundled Ofox assistant runtime

This is a private JSONL sidecar, not a standalone Pi installation. Ofox ships an
isolated production dependency tree with Pi SDK **1.4.0** and a SHA-256-verified
Node **22.23.4** (LTS). It does not require or discover the user's Node, Pi, CLI
tools, `~/.pi`, extensions, skills, prompt templates or MCP servers. Its system
prompt replaces Pi's coding-assistant role with a general chat assistant.

This version is chat only: the model receives no tools. `tools()` in
`src/runtime.mjs` is the extension point for later tools; they are built with
`wrapTool()` so they share the per-run call limit and the unknown-outcome gate.
The system prompt describes only what the run enables: a tool adds its own
sentences through Pi's `promptGuidelines`, so chat mentions no tools,
attachments or search.

## Prepare and test

From the repository root (build hosts need npm and tar; users do not):

```sh
pnpm assistant:install  # test dependencies in assistant-runtime/node_modules
pnpm assistant:test     # runtime tests on the pinned Node, mock providers only
pnpm assistant:prepare  # production resources for the current OS/architecture
pnpm assistant:smoke    # run the copied production tree: Pi SDK + native JSONL
```

`assistant:prepare` downloads the Node archive named in `node-downloads.json`
(cached in `.cache/`, rejected unless its SHA-256 matches), copies the `node`
executable and its LICENSE to `resources/node/`, installs the production
dependencies from `package-lock.json` into `resources/runtime/` with
`npm ci --omit=dev --ignore-scripts`, and prunes Pi's `examples/` (a demo ships
a GPL-licensed `doom.wasm`) and every `*.map` file. `resources/manifest.json`
records the protocol, SDK and Node versions and `minimumMacOS`, read from
`src-tauri/tauri.conf.json` (`bundle.macOS.minimumSystemVersion`, 12.0). Node
22 supports macOS 11 and later. With `APPLE_SIGNING_IDENTITY` set on macOS,
every Mach-O file is signed (hardened runtime and the JIT entitlement for Node,
except for ad-hoc `-` signing).

Tauri's dev and build hooks run `assistant:prepare`, and the resource map ships
`resources/` as `assistant-runtime/` in the application: the host starts
`assistant-runtime/node/node` (`node.exe` on Windows) with
`assistant-runtime/runtime/src/main.mjs`. Generated files are ignored by Git;
the npm lockfile and the pinned archive hashes are committed. Release packages
are macOS Apple Silicon and Windows x64; macOS Intel is pinned for local builds
and Linux x64 for CI.

The protocol tests spawn `resources/node/node`. Without it they fail at once
and ask you to run `pnpm assistant:prepare`. Their mock gateway listens on an
ephemeral port, so test runs can overlap.

## Private stdio contract (version 1)

Each line is one JSON object. No human diagnostics are written to stdout.
Requests are `{id,method,params}`; replies are `{id,result}` or
`{id,error:{code,message}}`. `health` returns protocol and SDK versions. `load`
takes a trusted absolute `sessionFile`. `run` and `compact` take:

```text
sessionId, runId, operationId, sessionFile, model, apiKey,
apex (ofox.ai|ofox.io), baseUrl?, allowLocalGateway?, prompt,
editEntryId?, regenerateEntryId?, thinkingLevel?, instructions? (compact)
```

`apiKey` is the assistant's own Ofox API key. It is sent only in the
Authorization header, never in a URL, and is scrubbed from history, events,
results and diagnostics. Provider requests go only to `https://api.{apex}`;
`baseUrl` may differ only when the host sets `allowLocalGateway` for its debug
gateway, which must be `http://localhost:<port>`.

`model` contains id, advertised supportedEndpoints, contextWindow,
maxOutputTokens, inputModalities/supportsImages, reasoning, cost (USD per million
tokens) and pricingKnown. Endpoint paths such as `/v1/responses` are accepted.
Only advertised OpenAI Chat Completions, Responses and Anthropic routes are
enabled; Google-only models are unavailable. SDK and provider automatic retries
are disabled.

`stop` matches sessionId/runId/operationId. The host owns one sidecar process
per operation and the stable native session file, and ends the process after
the response. Pi native JSONL is the context authority. Edit and regenerate
branch on native entry IDs; regenerate resends the native user text and images.
Every new turn has a non-context `ofox-desktop-run` marker with the run and
operation IDs and the historical pricing-availability flags. Projection entries
return normalized text, active branch, runId/operationId, usage, stopReason,
pricingKnown and final (only the last assistant entry of each run). Images and
system text are excluded from the projection. A turn that fails or is stopped
before its model request is kept in native history and can be retried
explicitly.

Events are `{method:"event",params:{sessionId,runId,operationId,type,data}}`.
Types include run_start/text_delta/thinking_delta/message_end/usage/
compaction_start/compaction_end/error/run_end (tool_start/tool_end once tools
exist). Streamed text holds back a tail long enough to scrub a split credential.
The host must persist the final RPC projection **before** publishing its UI
run_end. message_end is a runtime summary, not the application's message object.

Host tools, once registered, call `{id:"tool-...",method:"tool",params:{
sessionId,runId,operationId,toolCallId,name,args}}`. A host reply with
`error.code:"unknown_outcome"` or `error.data:{outcomeUnknown:true}` ends the
run without replaying an uncertain paid or mutating operation. Identical
tool-call IDs are deduplicated within a run and conflicting arguments are
rejected.
