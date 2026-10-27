# Bundled Ofox assistant runtime

This is a private JSONL sidecar, not a standalone Pi installation. Ofox ships an
isolated production dependency tree with Pi SDK **1.4.0** and a SHA-256-verified
Node **22.23.4** (LTS). It does not require or discover the user's Node, Pi, CLI
tools, `~/.pi`, extensions, skills, prompt templates or MCP servers. Its system
prompt replaces Pi's coding-assistant role with a general chat assistant.

The model receives a tool only when the run enables it: `read_attachment`
exists only when the conversation has attachments (see
[Attachments](#attachments)). `tools()` in `src/runtime.mjs` is the extension
point; tools are built with `wrapTool()` so they share the per-run call limit
and the unknown-outcome gate. The system prompt describes only what the run
enables: a tool adds its own sentences through Pi's `promptGuidelines`, so a
run without attachments mentions no tools, attachments or search.

## Prepare and test

From the repository root (build hosts need npm and tar; users do not):

```sh
pnpm assistant:install  # test dependencies in assistant-runtime/node_modules
pnpm assistant:test     # runtime tests on the pinned Node, mock providers only
pnpm assistant:prepare  # production resources for the current OS/architecture
pnpm assistant:smoke    # run the copied production tree: Pi SDK, native canvas,
                        # PDF text and rendering, native JSONL
```

`assistant:prepare` downloads the Node archive named in `node-downloads.json`
(cached in `.cache/`, rejected unless its SHA-256 matches), copies the `node`
executable and its LICENSE to `resources/node/`, installs the production
dependencies from `package-lock.json` into `resources/runtime/` with
`npm ci --omit=dev --ignore-scripts`, and prunes what the sidecar never loads
(`scripts/assistant-prune.mjs`): Pi's `examples/` (a demo ships a GPL-licensed
`doom.wasm`), every `*.map` file, and PDF.js's modern `build/` (it fails on
Node 22), `web/` viewer and `standard_fonts/` (read only through
`standardFontDataUrl`, which the runtime never sets). The PDF.js `legacy/`
build and the `cmaps/`, `iccs/` and `wasm/` it reads stay. `notices/` is
copied to `resources/notices/`: third-party notices for `@napi-rs/canvas`
(including the libraries compiled into its native addon) and `pdfjs-dist`.
`resources/manifest.json`
records the protocol, SDK and Node versions and `minimumMacOS`, read from
`src-tauri/tauri.conf.json` (`bundle.macOS.minimumSystemVersion`, 12.0). Node
22 supports macOS 11 and later, and the `@napi-rs/canvas` 1.0.13 addon macOS
11.0. With `APPLE_SIGNING_IDENTITY` set on macOS, every Mach-O file, the canvas
addon included, is signed (hardened runtime and the JIT entitlement for Node,
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
attachmentRoot?, attachments?[], selectedAttachmentIds?[], originalPrompt?,
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
branch on native entry IDs. Regenerate with `originalPrompt` rebuilds the turn
from that text and `selectedAttachmentIds` (see [Attachments](#attachments));
without it, it resends the native user text and images as stored. Every new turn has a non-context `ofox-desktop-run` marker with the run and
operation IDs and the historical pricing-availability flags. Projection entries
return normalized text, active branch, runId/operationId, usage, stopReason,
pricingKnown and final (only the last assistant entry of each run). Images and
system text are excluded from the projection; a user entry's content is the
text sent to the model, inlined attachment blocks included, so the host keeps
the user's own text and attachment selection. A turn that fails or is stopped
before its model request is kept in native history, with the native images it
was replaying, and can be retried explicitly.

Events are `{method:"event",params:{sessionId,runId,operationId,type,data}}`.
Types include run_start/attachment/text_delta/thinking_delta/tool_start/
tool_end/message_end/usage/compaction_start/compaction_end/error/run_end. Streamed text holds back a tail long enough to scrub a split credential.
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
  and every selected attachment is read again. Without it the stored native
  turn is replayed and the selection is ignored, so nothing is inlined twice.

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
for a model with image input. Pages of a PDF that have no extractable text
need image input: the first two are rendered and attached; the line names the
rest for `read_attachment` with `page`. A message carries at most 10 images or
rendered pages.

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

For each selected attachment, after `run_start` and before any model request,
the sidecar emits `type:"attachment"` with
`data:{id, status, code, details}`:

| status              | code                               | meaning                                                                                                         | run continues |
| ------------------- | ---------------------------------- | --------------------------------------------------------------------------------------------------------------- | ------------- |
| `ready`             | `null`                             | fully read and sent                                                                                             | yes           |
| `partial`           | `assistant_attachment_partial`     | sent, but part was not read: text cut at the extraction limit, or PDF pages without text not attached as images | yes           |
| `unreadable`        | `assistant_attachment_unreadable`  | not valid UTF-8, a damaged or password-protected PDF, a damaged image, or an unsafe or unreadable path          | no            |
| `unreadable`        | `assistant_attachment_not_found`   | the id is not in `attachments`, or the file is gone                                                             | no            |
| `unsupported`       | `assistant_attachment_unsupported` | not a readable format                                                                                           | no            |
| `too_large`         | `assistant_attachment_too_large`   | over a size, page or image limit                                                                                | no            |
| `model_unsupported` | `assistant_vision_model_required`  | an image, or a PDF with pages without text, sent to a model without image input                                 | no            |

If any outcome blocks, every selected attachment is still reported, no model
request is made, the `error` event has `code:"input_preparation_failed"`, and
the result has `status:"error"` with `errorMessage` set to the first blocking
attachment's `<code>: <message>`. The user's text is kept as a failed native
turn; retry it by regenerating its assistant entry with `originalPrompt` and
the selection. (A replayed native turn whose images need a vision model fails
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
  `pdf_pages` (`maxPages`, `pageCount`), `message_images` (`maxImages`) or
  `rendered_page` (`maxBytes`, `page`).
- `model_unsupported`: an image has `mimeType`; a PDF has `pageCount` and
  `pagesWithoutText`.

### Limits

25 MiB per file, 10 MiB per image, 100 PDF pages, 16 Mi characters (UTF-16
code units) of extracted text per attachment, 20,000 characters per
`read_attachment` call, 10 images or rendered pages per message. Rendered
pages are at most 2000 pixels per side, 4 megapixels and 5 MiB of PNG; no OCR
text is invented. Extracted text is cached per run in a 32 Mi-character LRU.
