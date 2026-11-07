# Assistant document engines

The assistant creates Office files and PDFs with two bundled command-line
engines. `pnpm assistant:prepare` downloads, verifies, signs and stages them
under `assistant-runtime/resources/engines/`, which the application ships as
`assistant-runtime/engines/` in its resources.

| Engine    | Version | Used for                         | What it is                                                            |
| --------- | ------- | -------------------------------- | --------------------------------------------------------------------- |
| officecli | 1.0.160 | Word, Excel and PowerPoint files | A self-contained, trimmed .NET 10 single-file program (not NativeAOT) |
| typst     | 0.15.1  | PDF from Typst markup            | A Rust program, no runtime and no JIT                                 |

They add to the package roughly: officecli about 34 MB (its download is the bare
binary), typst 14 to 22 MB compressed depending on the platform, and about 75 MB
together once unpacked (macOS arm64: officecli 34 MB, typst 45 MB).

Both are Apache-2.0. Their licences and notices ship in
`resources/notices/officecli.txt` and `typst.txt` and next to each executable
(`LICENSE`, `NOTICE`); the smoke test fails if either notice is missing.

## Sources and verification

Everything is pinned in `assistant-runtime/engine-downloads.json`: the release
URL and SHA-256 of each platform build. `assistant-prepare.mjs` refuses a
download whose digest differs, caches verified archives in
`assistant-runtime/.cache`, and writes `resources/engines/source.json` so an
unchanged engine is skipped on the next run.

- officecli: [iOfficeAI/OfficeCLI v1.0.160](https://github.com/iOfficeAI/OfficeCLI/releases/tag/v1.0.160).
  The digests are copied from the release's `SHA256SUMS`. The release ships a
  bare binary, so its `LICENSE` and `NOTICE` are pinned separately from the
  repository at the same tag.
- typst: [typst/typst v0.15.1](https://github.com/typst/typst/releases/tag/v0.15.1).
  Typst publishes no checksum file; each archive was downloaded once over TLS
  and its digest pinned (trust on first verified download). The archive carries
  its own `LICENSE` and `NOTICE`.

Builds exist for darwin-arm64, darwin-x64, linux-x64 and win32-x64. The
released platforms are macOS arm64 and Windows x64. To bump an engine, change
the version, URLs and digests together, then re-run the smoke test below.

Both engines keep state under `HOME`, and officecli can start a resident
process or update itself. `assistant-prepare.mjs` and `assistant-smoke.mjs`
therefore run them with a throwaway `HOME` (and `USERPROFILE`, `TMPDIR`, `TEMP`
and `TMP` in the same folder), `OFFICECLI_NO_AUTO_RESIDENT=1` and
`OFFICECLI_SKIP_UPDATE=1`, and stop an engine run after 60 seconds; the host must
do the same.

## macOS signing

Notarization needs every Mach-O inside the application to be signed with the
same Developer ID, with the hardened runtime and a secure timestamp.
`signMacFiles` in `scripts/assistant-prepare.mjs` does this for everything under
`resources/` when `APPLE_SIGNING_IDENTITY` is set, and replaces upstream
signatures: officecli arrives signed by its author's team (`52JQX2HUSC`) and
typst only ad-hoc (linker-signed).

Entitlements are chosen per executable name in `scripts/assistant-entitlements.mjs`:

| Executable                              | Entitlements file                                | Keys                              |
| --------------------------------------- | ------------------------------------------------ | --------------------------------- |
| `node`                                  | `assistant-runtime/node-entitlements.plist`      | `com.apple.security.cs.allow-jit` |
| `officecli`                             | `assistant-runtime/officecli-entitlements.plist` | `com.apple.security.cs.allow-jit` |
| `typst`                                 | none (explicit `null`)                           | hardened runtime only             |
| any Mach-O outside `resources/engines/` | none                                             | hardened runtime only             |

The table lists every engine executable on purpose. A Mach-O under
`resources/engines/` with no entry (a new engine, or an upstream rename) fails
`signMacFiles` with a descriptive error instead of being signed with the
hardened runtime and no entitlements, which would only crash when first run.
`tests/guards/assistantEntitlements.test.ts` also checks that every executable
`engine-downloads.json` pins for macOS has an entry.

### Why officecli gets `allow-jit` only

officecli is a .NET single-file program; the runtime JIT-compiles code, which
the hardened runtime forbids without `allow-jit`. Nothing here asks for more:

- The upstream v1.0.160 macOS arm64 build, signed by its author with the
  Developer ID of team `52JQX2HUSC` and the hardened runtime, carries exactly
  `com.apple.security.cs.allow-jit` and runs.
- The single-file bundle holds 59 managed assemblies and no native-binary
  entries, and nothing is extracted when it runs (review of the pinned binary).
  `otool -L` lists only Apple system libraries and the system Swift runtime, so
  there is no library of another team for library validation to reject.
- The signed, narrowed binary ran every officecli operation exercised below.

`disable-library-validation` would only remove the check on which libraries a
process may load, in a process that reads documents the agent was given, so it
is not granted. Without `allow-jit`, a hardened officecli fails at start-up
(`Failed to create CoreCLR, HRESULT: 0x80070008`, seen on a scratch copy).

### Spike result (2026-11-07, macOS 26, arm64)

Signed with `Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)`. The
output below is abridged.

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:prepare
codesign -dv --entitlements - assistant-runtime/resources/engines/officecli/officecli
```

```text
Identifier=officecli
CodeDirectory v=20500 size=66453 flags=0x10000(runtime) hashes=2066+7 location=embedded
TeamIdentifier=R65NVCLBMY
[Key] com.apple.security.cs.allow-jit    [Bool] true
```

typst reports `flags=0x10000(runtime)`, `TeamIdentifier=R65NVCLBMY` and no
entitlements, and Node keeps `allow-jit`. `codesign --verify --strict` passes
for both engines.

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:smoke
```

```text
Production smoke passed (v22.23.4; Pi SDK, native canvas, PDF text and rendering, native JSONL, search server, officecli 1.0.160, typst 0.15.1)
```

The smoke test runs the signed engines from the prepared resources:
`officecli --version`, `create` and `add` of a DOCX, `typst --version` and a
PDF compile. It passes with `allow-jit` alone.

Beyond the smoke test, the signed officecli ran this, with a throwaway `HOME`
and `TMPDIR`, `OFFICECLI_NO_AUTO_RESIDENT=1` (unset for the resident test) and
`OFFICECLI_SKIP_UPDATE=1`: create DOCX,
PPTX and XLSX; add a table and a picture to the DOCX, a picture and two charts
to the PPTX, and a chart and a picture to the XLSX; an 11-command `batch`;
`validate` on all three files; `view` as html (all three), svg (PPTX; officecli
supports no other format), `issues --json` and `stats --json`; and a resident
process started with `open` and stopped with `close` on an XLSX and a PPTX,
with edits made while it was open. Every operation succeeded and the saved files
validated. The unified log (`/usr/bin/log show --predicate 'process == "officecli"'`)
held only routine activity entries and no error or fault entries; `syspolicyd`
logged only its routine Gatekeeper assessment of the newly signed executable,
there were no kernel code-signing or library-load messages, and no crash
reports. Not exercised: `view screenshot`, `view pdf` and `watch`.

Gatekeeper rejects the staged executables (`spctl`: `Unnotarized Developer ID`)
until the application is notarized. That is expected.

### When the signed engine fails

Escalate only on a failure observed with the signed binary (killed on launch,
`Code Signature Invalid`, `library load disallowed`, an mmap or exec denial),
one step at a time, and record which step fixed it:

1. `com.apple.security.cs.allow-jit` (the current set).
2. Add `com.apple.security.cs.allow-unsigned-executable-memory`.
3. Add `com.apple.security.cs.disable-library-validation`.
4. Add `com.apple.security.cs.allow-dyld-environment-variables`.

If step 4 still fails, one proposal (not implemented or verified) is to ship no
officecli on macOS and install it on first use through the existing
`install_tool` flow, checking the download against the release's `SHA256SUMS`
and keeping it in the application data directory. All three Office formats
would stay available, and typst is unaffected because it needs no JIT.

### Notarization (to do, by the release owner)

Not done in this spike: `APPLE_ID`, `APPLE_PASSWORD` (app-specific password)
and `APPLE_TEAM_ID` are not available on the development machine. Notarization
covers the whole application, including these engines, so it is verified by a
normal release: either through the release workflow with its secrets or locally
with `scripts/release.sh` after exporting the three variables. A notarization
run that reaches `Accepted` from `notarytool`, followed by `spctl -a -vv` on the
installed application and one run of the engines from the installed bundle,
closes the spike. Both executables were signed with a secure timestamp and
target macOS 12.0 or earlier (officecli `minos` 12.0, typst 11.0), within the
application's 12.0 minimum.

## Windows

Nothing extra. Windows has no hardened runtime or entitlements. The engines are
bundled as `officecli.exe` and `typst.exe`, exactly as downloaded and verified
against the digests above. The Windows installer is not code-signed yet (see
the release workflow), so the engines are not either.

## Smoke test

`pnpm assistant:smoke` fails if a manifest entry for either engine is missing,
if either engine does not report its pinned version, if officecli cannot create
and edit a DOCX, if typst cannot compile a PDF, or if an engine notice is
missing.

CI and the branch-package workflow run it after an unsigned
`assistant-prepare` (no `APPLE_SIGNING_IDENTITY`), so they do not exercise the
hardened runtime or the entitlements. The release workflow signs the engines
(`pnpm tauri build` runs `assistant:prepare` with the Developer ID) but has no
signed smoke step. Until one exists, the signed run above, repeated on the
signed tree after any engine or entitlement change, is the only check.

## Using the engines: `create_document`

The model makes a file with one tool, `create_document({ name, format, content })`.
The runtime registers it from the run's `documents` parameter, which lists the
formats this computer can make: the text formats always, the Office formats when
officecli is present and `pdf` when typst is. The system prompt is written from
that list: it tells the model to use the tool when the user asks for a file or
names one of the PDF, Word, PowerPoint and Excel formats this computer can make.
Only when all four are available does it add that the model must never say it
cannot attach a file; otherwise it names the formats this machine cannot
produce, which the model says if asked.

Before the first document of a format in a conversation the model reads the
matching bundled skill with `use_skill`: `ofox-office-docx`, `ofox-office-pptx`,
`ofox-office-xlsx` or `ofox-office-pdf`. The skills hold the command reference
and layout rules; the three Office skills are adapted from the OfficeCLI
repository and the PDF skill is original (see
`assistant-runtime/notices/officecli-skills.txt`).

### Formats and content

| `format`                           | `content`                                 | What the host does                                                                                                 |
| ---------------------------------- | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------ |
| `pptx`, `docx`, `xlsx`             | an array of officecli batch commands      | `officecli create`, then `batch --input --json` (atomic: one failure rolls it all back), `validate`, `view issues` |
| `pdf`                              | the Typst markup of the body, as a string | joins the host's prelude and the body into `main.typ`, `typst compile`, then renders page 1 as a preview           |
| `md`, `txt`, `csv`, `html`, `json` | the file's text, as a string              | writes the file as given                                                                                           |

An Office batch is a list of `add`, `set`, `remove`, `move` and `swap` commands
(officecli's `op` is accepted as an alias of `command`). One batch makes one
file; a deck starts like this:

```json
[
  {
    "command": "add",
    "parent": "/",
    "type": "slide",
    "props": { "title": "Plan" }
  }
]
```

In the prose props `text` and `notes`, a literal `\n` (a backslash and an
n, as a model writes one when it escapes twice) is read as a newline, the way
officecli's own command line reads it; `batch` alone would keep it as written.

and a Word document can start a landscape section with a break:

```json
[
  {
    "command": "add",
    "parent": "/body",
    "type": "section",
    "props": { "type": "nextPage", "orientation": "landscape" }
  }
]
```

A PDF body starts at the text: the prelude already sets A4 pages with
margins and page numbers, a font chain that covers Chinese, Japanese and Latin
text, heading styles and the document language (the interface language).
Compile errors come back with their line and column in the body the model wrote
(the prelude's lines are not counted) and the quoted source line.

### Where the files live

- The finished file is kept as a generated attachment of the reply, under the
  conversation's private folder in the application's config directory:
  `assistant/<owner>/<session id>/attachments/<attachment id>`.
- The reply shows a document card with the file's name, size, page, slide or
  sheet count and a badge for issues. **Save and open** asks where to save, then
  opens the copy the user saved (never the stored one); **Save…** only saves.
- While a document is made the host works in `<session id>/outputs/<docId>/`
  (mode 0700): the content (`batch.json` or `main.typ`), the engine's private
  `.home` (its home and temporary folder) and the output and preview files. The
  folder stays for the tool call, because the runtime reads the preview from it
  after the host has answered, and is removed when the run ends, whether the run
  completed, was stopped or failed. A folder that cannot be removed is logged and
  never fails the run.

### Limits

| What                         | Limit                                                                  |
| ---------------------------- | ---------------------------------------------------------------------- |
| Documents per reply          | 3 (a refused or failed document does not count)                        |
| `content`, serialized        | 2 MiB                                                                  |
| Commands in one Office batch | 2000                                                                   |
| File name                    | 120 characters; folders are dropped and the format's extension is kept |
| One generated file           | 25 MiB; a conversation's files together 500 MiB                        |
| One engine run               | 60 s, then it and everything it started are killed                     |
| Engine output                | 256 KiB each of stdout and stderr                                      |
| Issues in a result and card  | 20, errors first, repeated ones counted together                       |
| Retries                      | the model retries a failed document at most twice, then explains       |

### What the host refuses to run

Office content is checked against an allowlist before officecli starts; a
refusal names the item and what to change, and nothing is made.

- Commands: only `add`, `set`, `remove`, `move` and `swap`. Every other
  officecli command (`raw`, `raw-set`, `add-part`, `view`, `get`, `query`,
  `import`, `dump`, `validate`) and any field a command does not take is refused.
- Elements: a path starts at a known root (`/body`, `/header`, `/footer`,
  `/slide[n]`, `/notes`, a sheet the batch itself adds, and so on). Diagrams, OLE
  objects, embedded objects and packages cannot be added.
- Values, by the property's key: a file property (`src`, `path`, `image`,
  `preview`, `poster`, ...) takes a `data:` URI and nothing else, so local and
  remote pictures are out; a link takes `https:`, `http:`, `mailto:` or a slide;
  a table's `data` is inline CSV; fills take colours, gradients or a `data:`
  image; prose (`text`, `title`, `notes`, `alt`, `description`, `tooltip`,
  `value`, `formula`) may hold anything, including addresses; every other value
  may not name a file or an address (an absolute or `..` path, `~`, a drive, a
  scheme).
- Word fields only show a page number or a date: `page`, `numpages`, `date`,
  `time` and `toc`, with no instruction that runs, fetches or links.
- Excel formulas (any key with `formula` in its name, `ref`, `refersTo` and
  `calculatedField`; a value that starts with `=`, `+`, `-` or `@` and is not a
  number; and every key of a conditional format, validation, named range,
  chart, sparkline, table or pivot table but its text keys, such as titles,
  names and prompts, and its inline lists, which are checked cell by cell
  unless they are one reference or hold `!` or `|`, when they are read whole
  as a formula of references) call only allowlisted functions, refuse
  WEBSERVICE, IMAGE, INDIRECT, DDE and the like even in quoted text, and refer
  to no other workbook, file share or address (HYPERLINK may link to one quoted
  `https:`, `http:`, `mailto:` or `#` address). A table's inline rows hold
  values, not formulas. Formulas are filtered, not sandboxed: Excel evaluates
  what passes when the workbook is opened (see
  [known limitations](known-limitations.md#assistant-document-engines-2026-11-07)).

The engines then run as isolated children: a clean environment, `HOME`,
`USERPROFILE`, `TMPDIR`, `TEMP` and `TMP` pointing into the document's `.home`,
`OFFICECLI_NO_AUTO_RESIDENT=1`, `OFFICECLI_SKIP_UPDATE=1`, a Typst package
folder that is empty and read-only,
every proxy variable pointing at a port nothing listens on, and no inherited
credentials. On macOS each engine also runs under `sandbox-exec` (probed per engine
until a probe passes; while it fails the engine runs without it, the next document
probes again, and the host logs the first failure): no
network for either engine; officecli writes only the document's folder and its own
temporary folder, and reads nothing under `/Users` and `/Volumes` except the
document's folder and its own. Windows
and Linux have the validator and the dead proxy only (see
[known limitations](known-limitations.md#assistant-document-engines-2026-11-07)).

### Previews

The model sees a preview of the first page only if it can see images. A PDF's
preview is Typst's PNG of page 1; a PowerPoint's is officecli's SVG of slide 1,
which the runtime draws with `@napi-rs/canvas`. Both become a JPEG of at most 768
px on the long edge and 200 KB, at a lower quality if needed and left out if it is
still too large, so a tool result stays well under the 1 MiB line limit. Word and
Excel files have no preview. The model also gets the file's id, name, type,
size, page, slide or sheet count and the issues; it never gets a file path.

### Not in this round

Editing or inspecting an existing Office file (`inspect_document` and
`edit_document` in the design) is not built: the model can only make new files.

## Acceptance on a real key

This is the user's step, with a real Ofox key; the development lane never ran it.

1. `pnpm assistant:prepare`, then `pnpm tauri dev`. Open the assistant with your
   Ofox key set and a model that supports tools (and, to check the previews,
   images).
2. Ask each of these in a new conversation and wait for the reply to end:
   1. "写一个 pdf，介绍四川"
   2. "做一个 8 页的 Transformer 介绍 ppt"
   3. "做一张三科成绩表"
   4. "写一份 Word 版的周报"
3. For each: a document card appears under the reply with the right name,
   type, size and count (pages, 8 slides, sheets); an issues badge, if any, opens
   a readable list; the model's text describes the file without saying it cannot
   attach one.
4. Press **Save and open** and choose a folder. The file opens in the system app
   (Preview or a PDF reader, PowerPoint or Keynote, Excel or Numbers, Word or
   Pages) with no repair prompt, warning or blank page, and the content is what
   the card said. Open the saved copy twice to be sure it is not the stored file.
5. After each reply ends, the conversation's
   `<AppConfig>/assistant/<owner>/<session id>/outputs/` folder is empty (no
   `<docId>` folders are left).
6. Optional: ask for a document with a picture, a chart, and a Word section
   break to see how the model uses the skills.

Record the result here, one line per prompt, with the date, build and macOS version.

| Date        | Build | macOS | pdf | pptx | xlsx | docx | Notes |
| ----------- | ----- | ----- | --- | ---- | ---- | ---- | ----- |
| not yet run |       |       |     |      |      |      |       |
