# Bundled Ofox assistant runtime

This is a private JSONL sidecar, not a standalone Pi installation. Ofox ships an
isolated production dependency tree with Pi SDK **1.4.0** and verified Node
**24.22.1 LTS**. It does not require or discover the user's Node, Pi, CLI tools,
`~/.pi`, extensions, skills, prompt templates or MCP servers. The two system
prompts override Pi's coding-assistant role.

From the repository root:

```sh
pnpm assistant:install  # development/test dependencies in this package only
pnpm assistant:test    # pinned Node; temporary sessions and mock providers
pnpm assistant:prepare # isolated resources for the current OS/architecture
pnpm assistant:smoke   # copied production SDK, native canvas, PDF and JSONL
```

Tauri's dev/build hook prepares `resources/`. The explicit resource map puts
`assistant-runtime/node/node` (`node.exe` on Windows) and
`assistant-runtime/runtime/src/main.mjs` in the application resources. Generated
dependencies and binaries are ignored by Git; the npm lockfile and pinned Node
archive SHA-256 values are committed. Build hosts need npm and tar; users do not.
Production Apple signing signs Node and every native addon with the same
Developer ID and hardened runtime. Ad-hoc previews omit hardened runtime because
ad-hoc binaries have no shared Team ID. Official signing/notarization and native
Windows validation remain release gates.

## Private stdio contract (version 1)

Each line is one JSON object. No human diagnostics are written to stdout.
Requests are `{id,method,params}`; replies are `{id,result}` or
`{id,error:{code,message}}`. `health` returns protocol and SDK versions. `load`
takes a trusted absolute `sessionFile`. `run` and `compact` take:

```text
sessionId, runId, operationId, sessionFile, kind (general|configuration),
model, accessToken, apex (ofox.ai|ofox.io), prompt, memory,
attachmentRoot, attachments[], selectedAttachmentIds[], allowedWebUrls[],
originalPrompt? (trusted original user text when regenerating),
webSearch, editEntryId?, regenerateEntryId?, instructions?
```

`model` contains id, advertised supportedEndpoints, contextWindow,
maxOutputTokens, inputModalities/supportsImages, reasoning, cost (USD per million
tokens), and pricingKnown. Endpoint paths such as `/v1/responses` are accepted.
Only advertised OpenAI Chat Completions, Responses, and Anthropic routes are
enabled. Google-only routes are unavailable until a header-only OAuth transport
can be guaranteed. Provider requests go only to the chosen Ofox apex; credentials
are placed exclusively in Authorization, never URL query parameters.

`stop` matches sessionId/runId/operationId. The host owns one sidecar process per
operation and the stable native session file, and ends the process after the
response. Pi native JSONL is the context authority. Edit/regenerate branch using
native entry IDs; regenerate copies the native user text/images. Every new turn
has a non-context `ofox-desktop-run` marker containing run and operation IDs and
the historical pricing-availability flags. Projection entries return normalized
text, active branch, runId/operationId, usage, stopReason, pricingKnown, and final
(only the last assistant entry of each run). Images and system/memory text are
excluded from the application message projection. Interrupted runs are retained
and require an explicit user retry.

Events are `{method:"event",params:{sessionId,runId,operationId,type,data}}`.
Types include run_start/text_delta/thinking_delta/tool_start/tool_end/message_end/
usage/compaction_start/compaction_end/error/run_end. The host must persist the
final RPC projection **before** publishing its UI run_end. message_end is a
runtime summary, not the application's AssistantMessage object.

Host tools are `{id:"tool-...",method:"tool",params:{sessionId,runId,
operationId,toolCallId,name,args}}`. Host authorization must bind these IDs to
the current account, organization and permission epoch. Host replies may use
`error.code:"unknown_outcome"` or
`error.data:{outcomeUnknown:true}` to terminate the run without replaying an
uncertain paid/write operation. SDK/provider automatic retries are disabled.
Identical tool-call IDs are deduplicated within a run and conflicting arguments
are rejected. Rust additionally persists operation claims.

## Boundaries and limits

General assistants expose only imported attachment reading and optional search/
web reading. Configuration assistants additionally expose config_status,
bind_tool, set_tool_model, install_tool, update_tool, diagnose_tool and
run_command. Rust performs every privileged host action; Node does not interpret
the permission mode. Fully authorized host commands cannot provide a strong
guarantee against accessing unknown local secrets. Current OAuth token values,
common credential fields and recognizable key forms are scrubbed at the source
before SDK history, tool results, model context, events and diagnostics.

Attachments are limited to their imported root, reject symlinks/traversal and
are at most 25 MiB. UTF-8 text, DOCX/XLSX/PPTX and PDF extraction are bounded to
16 Mi characters (UTF-16 code units); read_attachment pages at most 20,000
characters. Extraction at that limit returns an explicit truncation warning,
including in the initial model prompt. Extracted text uses a 32 Mi-character
LRU cache. XLSX text labels each cell with its coordinate, preserving sparse
rows and columns, and identifies formula results as cached values. OOXML
decompression is bounded and XML entities/macros are never executed. PDF import
accepts at most 100 pages. Scanned PDFs require a vision model and render pages
to PNG, at most 2000 pixels per dimension, 4 megapixels and 5 MiB per page; no
OCR text is invented. A message sends at most 10 images/pages. Raw images are
at most 10 MiB. Very complex/encrypted/malformed documents may fail explicitly.

Search runs through the host's OAuth/price check. Static web reading accepts
only user-provided URLs or URLs in this conversation's trusted search results.
It enforces HTTPS/443, all DNS answers and connected IP, rechecks every redirect,
and uses no cookies/authentication headers or browser JavaScript. Limits are
15 seconds, 4 redirects, 2 MiB response and 20,000 output characters. Local,
private, reserved and Ofox API hosts are blocked. VPN/proxy fake-IP DNS can be
rejected by this policy. Authenticated pages and browser automation are outside
this implementation.

The entire desktop application requires macOS **13.5 or newer**, matching the
bundled Node runtime. macOS 12–13.4 cannot run this release. Current package targets are macOS
Apple Silicon and Windows x64; Linux x64 is used for CI tests. Additional CPU/OS
targets require a pinned Node archive/hash and native addon verification.

Older installed clients do not filter updates by the minimum macOS version.
Publishing a new binary cannot retroactively change their update logic. Before
release, the legacy update manifest and download entry points must be handled
so unsupported systems are not directed to overwrite a usable old installation.
