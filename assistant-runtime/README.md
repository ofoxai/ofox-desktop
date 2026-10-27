# Bundled Ofox assistant runtime

This is a private JSONL sidecar, not a standalone Pi installation. Ofox ships an
isolated production dependency tree with Pi SDK **1.4.0** and a SHA-256-verified
Node **22.23.4** (LTS). It does not require or discover the user's Node, Pi, CLI
tools, `~/.pi`, extensions, skills, prompt templates or MCP servers. Its system
prompt replaces Pi's coding-assistant role with a general chat assistant.

The model receives a tool only when the run enables it: `read_attachment`
exists only when the conversation has attachments (see
[Attachments](#attachments)), `web_search` and `web_fetch` only when the run
sets `webSearch` (see [Web search](#web-search)). `tools()` in
`src/runtime.mjs` is the extension point; tools are built with `wrapTool()` so
they share the per-run call limit and the unknown-outcome gate. The system
prompt describes only what the run enables: a tool adds its own sentences
through Pi's `promptGuidelines`, so a run without attachments or web search
mentions no tools, attachments or search.

## Prepare and test

From the repository root (build hosts need npm and tar; users do not):

```sh
pnpm assistant:install  # test dependencies of the runtime and of search-mcp/
pnpm assistant:test     # runtime tests on the pinned Node, mock providers only,
                        # then search-mcp/'s build and tests
pnpm assistant:prepare  # production resources for the current OS/architecture
pnpm assistant:smoke    # run the copied production tree: Pi SDK, native canvas,
                        # PDF text and rendering, native JSONL, search server
```

`assistant:prepare` downloads the Node archive named in `node-downloads.json`
(cached in `.cache/`, rejected unless its SHA-256 matches), copies the `node`
executable and its LICENSE to `resources/node/`, installs the production
dependencies from `package-lock.json` into `resources/runtime/` with
`npm ci --omit=dev --ignore-scripts`, and prunes what the sidecar never loads
(`scripts/assistant-prune.mjs`): Pi's `examples/` (a demo ships a GPL-licensed
`doom.wasm`), every `*.map` file, and from PDF.js the modern `build/` (it
fails on Node 22), the `web/` and `legacy/web/` viewers, the standalone
`image_decoders/` and `legacy/image_decoders/` (the legacy build imports
neither) and `standard_fonts/` (read only through `standardFontDataUrl`,
which the runtime never sets). The PDF.js `legacy/build/` and the `cmaps/`,
`iccs/` and `wasm/` it reads stay. `resources/runtime-install.json` records the
lockfile and `package.json` hashes, a hash of `scripts/assistant-prepare.mjs`
and `scripts/assistant-prune.mjs`, Node version and platform of that install;
while they are unchanged the next prepare skips `npm ci` and only copies `src/`
and `notices/` again. A changed install or prune rule changes that hash, so
the next prepare installs and prunes afresh.

Prepare then builds the search MCP server (`search-mcp/`, see
[Web search](#web-search)): `npm ci --ignore-scripts` with its
devDependencies (esbuild) and `node build.mjs` on the bundled Node, and copies
the single file `dist/ofox-search-mcp.mjs` to `resources/search-mcp/`.
`resources/search-mcp-build.json` records its lockfile, `package.json`, build
script, sources and this script; while they are unchanged the build is
skipped.

`notices/` is copied to `resources/notices/`: third-party notices for
`@napi-rs/canvas` (including the libraries compiled into its native addon),
`pdfjs-dist` and the packages compiled into the search server
(`ofox-search-mcp.txt`; the search server's tests fail when the bundle gains a
package it does not name). `resources/manifest.json` records the protocol, SDK
and Node versions, the entry and search server paths and `minimumMacOS`, read
from `src-tauri/tauri.conf.json` (`bundle.macOS.minimumSystemVersion`, 12.0).
Node 22 supports macOS 11 and later, and the `@napi-rs/canvas` 1.0.13 addon
macOS 11.0. With `APPLE_SIGNING_IDENTITY` set on macOS, every Mach-O file, the
canvas addon included, is signed (hardened runtime, a secure timestamp and the
JIT entitlement for Node, except for ad-hoc `-` signing).

Tauri's dev and build hooks run `assistant:prepare`, and the resource map ships
`resources/` as `assistant-runtime/` in the application: the host starts
`assistant-runtime/node/node` (`node.exe` on Windows) with
`assistant-runtime/runtime/src/main.mjs`, and the sidecar starts
`assistant-runtime/search-mcp/ofox-search-mcp.mjs` with the same Node.
Generated files are ignored by Git; the npm lockfiles and the pinned archive
hashes are committed. Release packages are macOS Apple Silicon and Windows
x64; macOS Intel is pinned for local builds and Linux x64 for CI.

The protocol tests spawn `resources/node/node`. Without it they fail at once
and ask you to run `pnpm assistant:prepare`. Their mock gateway listens on an
ephemeral port, so test runs can overlap.

## Private stdio contract (version 1)

Each line is one JSON object. No human diagnostics are written to stdout.
Requests are `{id,method,params}`; replies are `{id,result}` or
`{id,error:{code,message}}`. `health` returns protocol and SDK versions. `load`
takes a trusted absolute `sessionFile`. `run` and `compact` take:

```text
sessionId, runId, operationId, sessionFile, model, apiKey, knownSecrets?,
apex (ofox.ai|ofox.io), baseUrl?, allowLocalGateway?, proxy?, prompt,
attachmentRoot?, attachments?[], selectedAttachmentIds?[], originalPrompt?,
webSearch?, allowedWebUrls?[], editEntryId?, regenerateEntryId?,
thinkingLevel?, instructions? (compact)
```

`apiKey` is the assistant's own Ofox API key. It is sent only in the
Authorization header, never in a URL. `knownSecrets` lists the run's earlier
keys (one the gateway refused before a retry). Conversation content (the
prompt, the transcript sent to the model, deltas, native history, projections
and stdout frames) passes byte for byte except for these exact keys, which
become `[REDACTED]`: code and config such as `api_key=os.environ[...]` or
`max_tokens: 4096` are the user's words. Diagnostics (stderr, `errorMessage`,
error frames) are also scrubbed of anything that looks like a credential. Provider requests go only to `https://api.{apex}`;
`baseUrl` may differ only when the host sets `allowLocalGateway` for its debug
gateway, which must be `http://localhost:<port>`.

`proxy` is `{url, noProxy}` or null. The host passes the app's own proxy
setting, else the system proxy as it resolves one for launched CLIs (a local
system proxy that does not answer means a direct connection), because the
sidecar gets no proxy environment variables. Requests then use undici
(pinned, the version Pi already depends on) with an `EnvHttpProxyAgent`: they
tunnel through the `http`, `https` or `socks5` proxy unless the target matches
`noProxy` (`NO_PROXY` syntax). The origin check above applies unchanged. The
URL may carry the proxy's credentials; it is never logged or echoed, and
diagnostics drop any `user:password@` part of a URL. The search server gets
the same proxy (see [Web search](#web-search)).

`model` contains id, advertised supportedEndpoints, contextWindow,
maxOutputTokens, inputModalities/supportsImages, reasoning, cost (USD per million
tokens) and pricingKnown. Endpoint paths such as `/v1/responses` are accepted.
Only advertised OpenAI Chat Completions, Responses and Anthropic routes are
enabled; Google-only models are unavailable. SDK and provider automatic retries
are disabled.

`stop` matches sessionId/runId/operationId. The host owns one sidecar process
per operation and the stable native session file, and ends the process after
the response. Pi native JSONL is the context authority. Edit and regenerate
branch on native entry IDs. Regenerate with `originalPrompt` rebuilds the turn
from that text and `selectedAttachmentIds` (see [Attachments](#attachments));
without it, a turn that failed before its request is rebuilt from the
attachment selection kept with it, and any other turn is resent as stored,
native images included. Every new turn has a non-context `ofox-desktop-run`
marker with the run and operation IDs and the historical pricing-availability flags. Projection entries
return normalized text, active branch, runId/operationId, usage, stopReason,
pricingKnown and final (only the last assistant entry of each run). A failed
run's result and its assistant entry also carry `errorStatus` (the HTTP status
of a refused model request) or `errorKind`: `not_sent` when the request never
left the machine (DNS, connection refused, connect timeout, TLS certificate, a
proxy that refused or failed before its tunnel to Ofox opened, or a turn that
failed before its request) and `network` when it was sent but no
answer came; `errorCode` names the network error. A 4xx or `not_sent` request
generated nothing, so its usage coverage is complete at zero cost; a `network`
or 5xx failure may have been billed and stays incomplete. Images and system
text are excluded from the projection; a user entry's content is the text sent
to the model, inlined attachment blocks included, so the host keeps the user's
own text and attachment selection. A turn that fails or is stopped before its
model request is kept in native history, with the native images it was
replaying and (in a non-context `ofox-desktop-attachments` entry) its
attachment selection, and can be retried explicitly.

Events are `{method:"event",params:{sessionId,runId,operationId,type,data}}`.
Types include run_start/attachment/text_delta/thinking_delta/tool_start/
tool_end/message_end/usage/compaction_start/compaction_end/error/run_end
(`tool_start`/`tool_end` of the web tools: see [Web search](#web-search)).
Streamed text holds back a tail long enough to replace a key split across
deltas.
The host must persist the final RPC projection **before** publishing its UI
run_end. message_end is a runtime summary, not the application's message object.

Host tools call `{id:"tool-...",method:"tool",params:{
sessionId,runId,operationId,toolCallId,name,args}}`. A host reply with
`error.code:"unknown_outcome"` or `error.data:{outcomeUnknown:true}` ends the
run without replaying an uncertain paid or mutating operation. Identical
tool-call IDs are deduplicated within a run and conflicting arguments are
rejected.

## Attachments

The host copies imported files into a private directory per conversation and
passes, on every `run`:

- `attachmentRoot`: that directory's absolute path. It may be a system alias
  such as macOS `/var` -> `/private/var`; nothing below it may be a symlink.
- `attachments`: up to 200 entries `{id, name, mimeType, sizeBytes, path}` for
  every file in the conversation. `id` is unique (at most 200 characters);
  `path` is relative to the root or absolute inside it; `name` is the original
  file name; `sizeBytes` is informational, the file itself is checked.
- `selectedAttachmentIds` (default none): the attachments the user sent with
  this turn, inlined into it in order; duplicates are read once.
- `originalPrompt`: with `regenerateEntryId`, the user's own text of the turn
  being regenerated. The turn is rebuilt from it and `selectedAttachmentIds`
  and every selected attachment is read again. Without it, a turn that failed
  before its request is rebuilt from its stored text and the selection kept
  with it, so a plain retry never drops its attachments; any other turn is
  replayed as stored and the selection is ignored, so nothing is inlined
  twice.

Malformed attachment parameters reject the request before a turn starts.

Formats are chosen by the name's extension, then the MIME type. Readable:
UTF-8 text (`.txt .md .csv .json .log .yaml .yml .toml .xml .html .js .ts .py
.rs .go`, `text/*`, `application/json`); PDF text (`.pdf`, `application/pdf`)
read with the PDF.js legacy build and its CMaps, so CJK text in predefined
CMaps is extracted; and PNG, JPEG, WebP and GIF images, identified by their
bytes. Other formats are unsupported.

Selected attachments are inlined under `[User attachment: "<name>";
id=<id>]`: the first 8,000 characters of text (the rest through
`read_attachment`, which the line names), any extraction warning, and images
for a model with image input. With image input, the first two pages of a PDF
that have no extractable text are rendered and attached (PNG, or JPEG when the
PNG exceeds 5 MiB); the line names the rest, and any page that failed to
render, for `read_attachment` with `page`. Without image input a PDF's text is
still sent and its outcome is `partial`; only a PDF without any extractable
text needs image input. A message carries at most 10 images or rendered pages.
Stop interrupts reading between PDF pages.

When the conversation has attachments the run registers `read_attachment`
`{id, offset?, length? (<= 20000), page?}`. It returns a text window with
`format, text, offset, nextOffset, hasMore, characters, truncated,
extractionLimit, warning?` and, for PDFs, `pageCount, scanned,
pagesWithoutText, textCoverage`. With image input it also returns an image
attachment, or one PDF page rendered to PNG with `page`; `page` is not offered
to a model without image input. Its prompt guidelines state exactly this,
including that a model without image input cannot read images or pages without
text. Failures reach the model and `tool_end.message` as `<code>: <message>`
with the codes below.

### Outcome events

For each selected attachment, as soon as it has been read (after `run_start`,
before any model request), the sidecar emits `type:"attachment"` with
`data:{id, status, code, details}`:

| status              | code                               | meaning                                                                                                                                         | run continues |
| ------------------- | ---------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------- | ------------- |
| `ready`             | `null`                             | fully read and sent                                                                                                                             | yes           |
| `partial`           | `assistant_attachment_partial`     | sent, but part was not read: text cut at the extraction limit, or PDF pages without text not attached as images (always so without image input) | yes           |
| `unreadable`        | `assistant_attachment_unreadable`  | not valid UTF-8, a damaged or password-protected PDF, a damaged image, or an unsafe or unreadable path                                          | no            |
| `unreadable`        | `assistant_attachment_not_found`   | the id is not in `attachments`, or the file is gone                                                                                             | no            |
| `unsupported`       | `assistant_attachment_unsupported` | not a readable format                                                                                                                           | no            |
| `too_large`         | `assistant_attachment_too_large`   | over a size, page or image limit                                                                                                                | no            |
| `model_unsupported` | `assistant_vision_model_required`  | an image, or a PDF without any extractable text, sent to a model without image input                                                            | no            |

If any outcome blocks, every selected attachment is still reported, no model
request is made, the `error` event has `code:"input_preparation_failed"`, and
the result has `status:"error"` with `errorMessage` set to the first blocking
attachment's `<code>: <message>`. The user's text and selection are kept with
a failed native turn; regenerating its assistant entry reads the kept
selection again, or the host's `selectedAttachmentIds` when it passes
`originalPrompt`. Stop during preparation ends the run as `stopped`; outcomes
of attachments not yet read are not reported. (A replayed native turn whose images need a vision model fails
the same way with `assistant_vision_model_required`, without an attachment
event.)

`details` fields:

- `format`: `"text"`, `"pdf"` or `"image"`, whenever the format is known.
- Text and PDF (`ready`, `partial`): `characters` (extracted), `inlineCharacters`,
  `hasMore` (more through `read_attachment`), `truncated` (the 16 Mi-character
  extraction limit was hit; unread content remains) and, when truncated,
  `extractionLimit`.
- PDF: `pageCount`, `textCoverage` (`complete`, `partial` or `none`),
  `pagesWithoutText` and, with image input, `pagesAttachedAsImages`.
- Image: `mimeType` (detected from the bytes).
- `unreadable`: `reason` is `unknown_id` or `missing` (code
  `assistant_attachment_not_found`), `unsafe_path` (outside the root, symlink,
  not a regular file, or changed while opening), `read_failed`, `invalid_utf8`,
  `invalid_pdf`, `password_protected` or `invalid_image`.
- `unsupported`: `extension` and the declared `mimeType`; an image type other
  than PNG, JPEG, WebP or GIF has `format:"image"` and `mimeType`.
- `too_large`: `limit` is `file_size` or `image_size` (`maxBytes`),
  `pdf_pages` (`maxPages`, `pageCount`), `message_images` (`maxImages`),
  `rendered_page` (`maxBytes`, `page`) or `pdf_image` (`maxPixels`, `page`).
- `model_unsupported`: an image has `mimeType`; a PDF (no extractable text on
  any page) has `pageCount` and `pagesWithoutText`.

### Limits

25 MiB per file, 10 MiB per image, 100 PDF pages, 16 Mi characters (UTF-16
code units) of extracted text per attachment, 20,000 characters per
`read_attachment` call, 10 images or rendered pages per message. Rendered
pages are at most 2000 pixels per side, 4 megapixels and 5 MiB (PNG, else
JPEG); no OCR text is invented. PDF.js does not decode an embedded image over
40 megapixels (a 600-dpi A4 scan has about 35), so a page holding one is not
rendered rather than sent blank, unless PDF.js stops at another error earlier
on the same page: inline it stays for `read_attachment`, which reports
`too_large` with `pdf_image` for it. Extracted text is cached per run in a
32 Mi-character LRU.

## Web search

With `webSearch: true` the run registers `web_search {query}` (1-2000
characters) and `web_fetch {url}` (`src/search.mjs`). Both are tools of the
bundled search MCP server ([`search-mcp/`](search-mcp/README.md), shipped as
`search-mcp/ofox-search-mcp.mjs` next to `runtime/`). The runtime starts it on
the first call with pi-mcp's `McpClient` + `StdioTransport` (`node`, the bundle
path as the only argument, `inheritEnv: false`) and stops it when the run
ends; a stop while it starts ends the wait at once, and it must answer
`initialize` within 10 s. The environment it is given is `OFOX_ASSISTANT_KEY`
(the run's `apiKey`, never on the command line), `OFOX_BASE_URL` (`baseUrl`,
else `https://api.{apex}`), `OFOX_ALLOW_LOCAL_GATEWAY=1` with
`allowLocalGateway`, `OFOX_PROXY_URL` and `OFOX_NO_PROXY` from `proxy`, and
`SystemRoot`, `TEMP`, `TMP` and `PATH` when the sidecar has them. The OS adds
its own on top: macOS `__CF_USER_TEXT_ENCODING`, and on Windows libuv copies
the variables every process needs (`USERPROFILE`, `WINDIR`, `USERNAME` and
others) from the sidecar. pi-mcp starts the server in its own process group on
POSIX, so it exits as soon as its stdin closes, even during a search: a
sidecar that dies, killed with its group or not, takes it along. The server's
own stderr (already redacted) is forwarded to the sidecar's stderr.

### Contract for the search host

Run parameters:

- `webSearch` (default off): registers both tools and their prompt guidelines
  (search only for current or outside information; search once with one
  precise query and again only if the results are insufficient, at most 3
  times; prefer the search summary to reading pages; cite the sources used as
  Markdown links). Compaction runs register them too but never call them.
- `allowedWebUrls` (default none): the HTTPS pages `web_fetch` may read,
  usually the URLs in the user's messages and the sources of earlier answers.
  At most 500 strings of at most 2048 characters; anything else rejects the
  request before a turn starts. Every source a `web_search` of the run returns
  is added. URLs compare as the server reads them: without the fragment, with
  a lower-case host and no default port. Other URLs fail with
  `fetch_not_allowed` without a request.
- `proxy`: the one model requests use. The server tunnels both tools through
  it; `web_fetch` checks DNS locally first and verifies the certificate for
  the host name (see the search-mcp README). When every local answer is a
  fake-IP address (`198.18.0.0/15`, Clash/Surge TUN), `web_fetch` checks the
  host with DNS-over-HTTPS at `https://dns.alidns.com/resolve` (a fixed
  infrastructure URL, not an Ofox one) through the same route as
  `web_search`, refuses it with `fetch_not_allowed` unless every answer there
  is public, and connects to a checked public address (through the proxy:
  tunnels to it), never to the fake-IP. Sites that only a TUN or proxy domain
  rule can reach may then fail unless it reads the name from the TLS SNI.

Events: `tool_start` is `{name, toolCallId, args}` with `args` `{query}` or
`{url}`. `tool_end` is `{name, toolCallId, success:true, details}` or
`{name, toolCallId, success:false, message, details}`:

| Tool         | Success `details`                                                                             | Failure `details` |
| ------------ | --------------------------------------------------------------------------------------------- | ----------------- |
| `web_search` | `{usage: {searchCalls, inputTokens, outputTokens, model}, sources: [{title, url, snippet?}]}` | `{code, status?}` |
| `web_fetch`  | `{url, truncated}`, `url` after redirects                                                     | `{code, status?}` |

`message` is `<code>: <message>`, the text the model reads. The codes are the
server's (see the search-mcp README) and the runtime's own:
`search_unavailable` (the server could not start, or stopped; nothing was
sent; a later call starts it again), `search_rejected`/`fetch_rejected` (the
server rejected the arguments), `search_failed` without `status` also when the
server answered with a JSON-RPC error (it never ran the search),
`fetch_not_allowed` (also: the URL is not allowed in this run), and the
run's budget: `search_limit_reached` (the run already sent 3 searches to the
server) and `fetch_limit_reached` (3 `web_fetch` calls of the run already
failed, refusals included). Both are ordinary failures without a request:
they tell the model to answer with what it has, and the run goes on.
Source URLs are passed on as the server returns them; `inputTokens` and
`outputTokens` are `null` when the gateway did not report them.

Billing: every `web_search` success is `usage.searchCalls` paid searches at
the web_search price of `usage.model`, plus its tokens. The server asks for
one search (`max_tool_calls: 1`), but the gateway does not guarantee it: one
`web_search` may bill 1-2 calls (a `search` and an `open_page`), so
`searchCalls` is usually 1 and can be 2. A price shown before a search is a
range, 1-2 times the web_search price plus tokens; a finished search is
priced from its `searchCalls`, never from an assumed 1. Nothing is retried
automatically. The same query again in one run (case and spacing ignored)
after a success, or while it runs, is answered from that result with
`deduplicated: true` and zero usage, also past the limit; after a failure the
model may try it again. A run sends at most 3 searches to the server, failed
ones included; repeats answered from an earlier result do not count. A
failure other than `search_outcome_unknown` cost nothing.

Unknown outcome: `search_outcome_unknown` comes from the server, or from the
runtime when the server did not answer within 70 s, answered malformed, the
run was stopped during the search or the server exited during it. Like a host
`unknown_outcome` it ends the run without another tool or model request:
`status:"error"` with `errorMessage` `search_outcome_unknown: ...`, or
`stopped` when the user stopped it.

Native history and projection: every tool entry (`role:"tool"`) has
`toolName`, `toolCallId` and `isError`. A web tool entry keeps `details`: the
success details above, `{code:"unknown_outcome", error:{code:
"search_outcome_unknown"}}` for an unknown outcome, `{}` for other failures. A
`web_search` entry has `usageCoverage`: `incomplete` for an unknown outcome or
missing token counts, else `complete`; the projection's `usageCoverage`
includes it. The host prices searches itself, so search usage is reported
only here and in `tool_end`, never in `usage` events or entries.
