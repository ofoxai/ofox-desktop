# Assistant document engines

The assistant creates Office files and PDFs with two bundled command-line
engines, and on macOS makes PowerPoint decks from Python code with a third.
`pnpm assistant:prepare` downloads, verifies, signs and stages them
under `assistant-runtime/resources/engines/`, which the application ships as
`assistant-runtime/engines/` in its resources.

| Engine    | Version                        | Used for                                      | What it is                                                                                                                            |
| --------- | ------------------------------ | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| officecli | 1.0.160                        | Word, Excel and PowerPoint files              | A self-contained, trimmed .NET 10 single-file program (not NativeAOT)                                                                 |
| typst     | 0.15.1                         | PDF from Typst markup                         | A Rust program, no runtime and no JIT                                                                                                 |
| python    | Pyodide 314.0.7 (CPython 3.14) | PowerPoint decks from Python code, macOS only | CPython compiled to WebAssembly, run by the runtime's own Node with bundled wheels and a CJK font ([below](#python-decks-pptx-macos)) |

They add to the package roughly: officecli about 34 MB (its download is the bare
binary), typst 14 to 22 MB compressed depending on the platform, and about 75 MB
together once unpacked (macOS arm64: officecli 34 MB, typst 45 MB).

officecli and typst are Apache-2.0. Their licences and notices ship in
`resources/notices/officecli.txt` and `typst.txt` and next to each executable
(`LICENSE`, `NOTICE`); the smoke test fails if either notice is missing. The
Python engine adds about 37 MB on macOS (Pyodide's package 13 MB, wheels 16 MB,
the font 8 MB); Pyodide is MPL-2.0 and its notice, with those of the wheels and
the font, is `resources/notices/python.txt`.

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

The Python engine is pinned in `scripts/python-downloads.json`: the Pyodide
version (the `pyodide` package in the runtime's `package-lock.json`), and for
each wheel its file name and SHA-256 (numpy, matplotlib and its dependencies,
lxml, Pillow, python-pptx, python-docx, openpyxl, XlsxWriter and the pure Python
packages they need), plus the Noto Sans SC font. `assistant-prepare.mjs`
downloads and verifies them like the engines above (`OFOX_PYODIDE_BASE_URL`
points the binary wheels at a mirror; the digests are still checked) and stages
them under `resources/engines/python/`, recorded in the manifest with the
runner script. Only the platforms listed in that file stage it (macOS today);
elsewhere nothing is staged and the `pyodide` package is pruned from the
runtime. Nothing is downloaded when a deck is made.

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
the release workflow), so the engines are not either. The Python engine is not
bundled on Windows or Linux: it runs only inside the macOS system sandbox, and
those platforms have no equivalent yet, so a PowerPoint there is always made
from a deck object.

## Smoke test

`pnpm assistant:smoke` fails if a manifest entry for either engine is missing,
if either engine does not report its pinned version, if officecli cannot create
and edit a DOCX, if typst cannot compile a PDF, or if an engine notice (including
`python.txt`) is missing. It also checks that the `pyodide` package ships where
the Python engine is offered and is pruned everywhere else.

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
produce, which the model says if asked. The tool's description and the prompt
mention Python code and previews only when the host offers the Python engine
(`documents.python`; see [Python decks](#python-decks-pptx-macos)): a computer
that cannot run it never hears of them.

Before the first document of a format in a conversation the model reads the
matching bundled skill with `use_skill`: `ofox-office-docx`, `ofox-office-pptx`,
`ofox-office-xlsx` or `ofox-office-pdf`. The skills hold the command reference
and layout rules; the three Office skills are adapted from the OfficeCLI
repository (the PowerPoint skill and the deck renderer take their page types
and palette from its `morph-ppt` skill) and the PDF skill is original (see
`assistant-runtime/notices/officecli-skills.txt`). The PowerPoint skill teaches
the Python route first (a slide plan, the helpers, design rules, the preview
step) and keeps the deck object as the fallback for a computer that does not
offer Python.

### Formats and content

| `format`                           | `content`                                                                        | What the host does                                                                                                                                                                                                                                                                                                 |
| ---------------------------------- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pptx`                             | Python deck code (macOS), a deck object, or an array of officecli batch commands | runs the Python code and takes the deck it saved ([below](#python-decks-pptx-macos)), or runs the batch the model wrote or the runtime rendered from a deck object ([below](#the-deck-object-pptx)); every route ends with the checks of docx and xlsx, [layout fixes](#layout-fixes) and one `view svg` per slide |
| `docx`, `xlsx`                     | an array of officecli batch commands                                             | `officecli create`, then `batch --input --json` (atomic: one failure rolls it all back), `validate`, `view issues`                                                                                                                                                                                                 |
| `pdf`                              | the Typst markup of the body, as a string                                        | joins the host's prelude and the body into `main.typ`, `typst compile`, then renders page 1 as a preview                                                                                                                                                                                                           |
| `md`, `txt`, `csv`, `html`, `json` | the file's text, as a string                                                     | writes the file as given                                                                                                                                                                                                                                                                                           |

An Office batch is a list of `add`, `set`, `remove`, `move` and `swap` commands
(officecli's `op` is accepted as an alias of `command`). One batch makes one
file; a raw PowerPoint batch starts like this:

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

#### Python decks (pptx, macOS)

Where the host offers the Python engine, the model's first choice for PowerPoint
is Python code: `content` is `{ "kind": "python", "code": "…" }` (the code as
one string of at most 200 000 bytes; the runtime also takes that object as JSON
text). A deck written this way is free in layout and holds plots and diagrams
drawn from the model's own numbers, where a deck object has ten fixed page types
and one picture, chart or table on a showcase slide; the deck object stays for
the computers that cannot run it.

- **What runs.** The host writes the code to `outputs/<docId>/python/code.py` and
  starts the runner (`assistant-runtime/src/python/runner.mjs`) with the
  runtime's own Node. The runner loads Pyodide and the bundled wheels (nothing is
  downloaded or installed) and runs the code with python-pptx, matplotlib and
  numpy at hand and the deck helpers `deck.py` already imported: a cover, a
  section break and a content page with a real title placeholder, and cards, key
  numbers, steps, a timeline, text, shapes, SVG figures and matplotlib plots.
  Chinese text in plots uses the bundled Noto Sans SC. The deck starts from
  `template.pptx`, a 16:9 template with a cover, a section and a content layout
  (title placeholder, footer, page number), built by
  `scripts/assistant-deck-template.mjs`. As they write, the helpers check how
  much room a text needs, how empty a box is, the smallest type in a figure and
  an SVG's proportions, and report what they find as issues.
- **What comes back.** The runner saves the deck itself; the code never does. A
  mistake in the code comes back as the end of its Python traceback, bounded, and
  nothing is made; the model fixes the line and sends the code again (at most two
  retries, as for any failed document).
- **How it is held.** Pyodide is not a security boundary, so the operating
  system holds the code. The host runs the runner as its own child process, under
  its own `sandbox-exec` profile (stricter than officecli's), with the clean
  environment, the private home and temporary folders and the dead proxy that the
  other engines get, in its own subfolder of the document's folder,
  `outputs/<docId>/python/`. The code cannot use the network, cannot start any
  other program, can write only that subfolder, and cannot read the user's other
  files or the shared temporary folders. It is stopped after 90 seconds
  (matplotlib builds its font cache in the first run), and when the user stops
  the reply the runner ends with it. Python runs are serialised, one at a time,
  since each takes hundreds of megabytes.
- **Fail closed.** Python is offered only when both probes pass: its own and
  officecli's, which checks the deck. If `sandbox-exec` is missing or either probe
  fails, the engine is off: the tool and the prompt say nothing of Python, and a
  call that sends Python code is refused with `send a deck object instead`.
  officecli and Typst, by contrast, run without a sandbox when their probe fails.
- **Checks.** The runner makes the chart axis ids positive (python-pptx writes
  negative ones, which the schema does not allow). The host copies the deck out of
  the Python subfolder as a regular file (a link, a folder or a file over 25 MiB
  is no deck), and officecli, in its own sandbox, checks it as it checks a
  batch's file: `validate`, `view issues`, up to two rounds of
  [layout fixes](#layout-fixes) and one SVG page per slide. The helpers' own
  lines are reported with officecli's issues, as warnings.

#### The deck object (pptx)

Where the host does not offer Python (Windows, Linux, a macOS whose sandbox
probes fail), the model's first choice for PowerPoint is a deck object, not
commands: `{ "kind": "deck", "title", "language", "theme", "slides": [...] }`;
where it does, a deck object is the fallback. The model
writes content only (text, lists, and one picture, chart or table on a showcase
slide). The runtime chooses fonts, sizes, colours and positions, renders the
slides with the `navy` theme into a batch, and that batch takes the same path as
one the model wrote: the allowlist check, the sandboxed engine run, `validate`,
`view issues` and the layout fixes. `theme` can be left out (`navy` is the only
one) and `language` is `zh`, `en` or `ja`, which picks the font chain.
A successful call says `Rendered N slides from the deck with theme navy.`
The runtime also takes the deck as a JSON string, without `kind` when it has a
`slides` list, or with `kind` in any case, and reads a literal `\n` or `\r\n`
in any of its text as a line break before it checks the limits, as the host does
in `text` and `notes` (a lone `\r`, as in `C:\reports`, is kept). A showcase's
first table header, first chart category and first series name may not start
with `@` or a path prefix (`/`, `./`, `../`, `~/`, a drive), since they start
the value the host checks as a whole. When the host refuses a batch the runtime
rendered, the model is told `The rendered batch was refused by the host (this is
a renderer problem, not your deck): ` before the host's words, so it does not
rewrite a deck that was fine.

A deck has 3 to 30 slides, each of one of ten page types, and any slide may
carry speaker `notes`:

| `type`       | For                         | Content                                                  |
| ------------ | --------------------------- | -------------------------------------------------------- |
| `hero`       | the cover                   | title, subtitle, meta (presenter, date)                  |
| `statement`  | one sentence to remember    | statement, note                                          |
| `pillars`    | 2 to 4 parallel points      | title, 2 to 4 of heading and body                        |
| `evidence`   | key numbers                 | title, 1 to 4 metrics (value, label, note), detail lines |
| `timeline`   | steps in order              | title, 3 to 6 of heading and body                        |
| `comparison` | two sides                   | title, left and right, each a heading and 1 to 4 points  |
| `grid`       | 3 to 8 short parallel items | title, 3 to 8 of heading and body                        |
| `quote`      | a quotation                 | quote, source                                            |
| `cta`        | the closing slide           | title, subtitle, contact                                 |
| `showcase`   | one picture, chart or table | title, media, caption, body                              |

Limits are counted in CJK units: a Chinese character is one unit, a Latin
letter, digit or space is half a unit, rounded up. Every field has its own
limit, line breaks are allowed only in a few body fields (at most 2) and in
`notes`, and a showcase's `media` holds exactly one of a `data:` image, a chart
(`bar`, `line` or `pie`) or a table. There is no `bullets` page type: a list is
`pillars` or `grid`. The renderer never truncates or rewrites the model's text;
a deck that breaks a rule is refused whole, with the message
`The deck was not rendered; fix these and send the whole deck again:` and one
problem per line, each naming the slide, the field and the limit. Nothing is
saved, and the model retries as for any failed document (at most twice).

The table of every limit, the narrative structures and a worked example are in
`assistant-runtime/skills/ofox-office-pptx/SKILL.md`, which is what the model
reads; the limits themselves are `LIMITS` in
`assistant-runtime/src/deck/schema.mjs`, so change the two together. The page
types, narrative structures and palette values come from the `morph-ppt` skill
of OfficeCLI (Apache-2.0; see `assistant-runtime/notices/officecli-skills.txt`).

A raw batch is still accepted for PowerPoint as the expert mode, for what no
page type does (a custom layout, a picture at a chosen spot). Word and Excel
always take batches. A raw PowerPoint batch gets the same checks, layout fixes
and slide pages as a rendered deck.

### Where the files live

- The finished file is kept as a generated attachment of the reply, under the
  conversation's private folder in the application's config directory:
  `assistant/<owner>/<session id>/attachments/<attachment id>`.
- The reply shows a document card with the file's name, size, page, slide or
  sheet count and a badge for issues. **Save and open** asks where to save, then
  opens the copy the user saved (never the stored one); **Save…** only saves.
- While a document is made the host works in `<session id>/outputs/<docId>/`
  (mode 0700): the content (`batch.json` or `main.typ`), the engine's private
  `.home` (its home and temporary folder) and the output and preview files; a
  Python deck's code and its runner's output are in the subfolder `python/`.
  The folder stays for the tool call, because the runtime reads the preview from
  it after the host has answered (and, for a
  [preview](#previewing-before-delivering), until the reply ends), and is removed
  when the run ends, whether the run completed, was stopped or failed. A folder
  that cannot be removed is logged and never fails the run.

### Layout fixes

After `batch`, `validate` and `view issues`, a PowerPoint gets up to 2 rounds of
automatic layout repair before its card is made, so a text box that is a little
too small does not send the model back to redo the whole file. For the issues
officecli reports, the host writes `set` commands (a round looks up where at
most 40 shapes are; the issues of shapes beyond those stay warnings):

| Issue         | Fix                                                                                                                                 |
| ------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| text overflow | the box grows to the height officecli suggests while it stays inside the page; otherwise the font steps down 2 pt, to a 13 pt floor |
| off the slide | the shape moves back to 1.27 cm inside the slide's edge                                                                             |
| low contrast  | the text turns white                                                                                                                |

Each fix batch goes through the same allowlist check and the same sandbox as a
batch the model wrote. A fix batch that officecli rejects is rolled back; one
that crashes or times out may leave the file half-changed. Either way the host
runs `validate` again: if it reports nothing, the document stands with the
issues of a fresh `view issues`; if it reports an error or gives no report, the
call fails. The host's result and the generation record carry `fixed`, the
number of `set` commands applied, and the runtime tells the model
`Auto-fixed N layout issue(s).` when it is more than 0. A rendered deck normally
has none to fix, because the renderer fits its own text.

### Limits

| What                         | Limit                                                                  |
| ---------------------------- | ---------------------------------------------------------------------- |
| Documents per reply          | 3 (a refused or failed document does not count)                        |
| `content`, serialized        | 2 MiB                                                                  |
| Python deck code             | 200 000 bytes                                                          |
| Previews of one file name    | 2 per reply; they do not count as documents                            |
| Commands in one Office batch | 2000                                                                   |
| File name                    | 120 characters; folders are dropped and the format's extension is kept |
| One generated file           | 25 MiB; a conversation's files together 500 MiB                        |
| One engine run               | 60 s, then it and everything it started are killed                     |
| One Python run               | 90 s, then it and everything it started are killed                     |
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
  a table's `data` is inline CSV (in a deck the whole value of `data` and of
  chart `categories` is checked, since officecli reads a file only when the
  whole value names one, and a value starting with a drive letter is refused
  unless numbers follow it up to the first `;`; in Word and Excel each cell is
  checked too); fills take colours, gradients or a `data:`
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
document's folder and its own. The Python engine is the exception to "runs
without it": it has its own, stricter profile and never runs without it (see
[Python decks](#python-decks-pptx-macos)). Windows
and Linux have the validator and the dead proxy only, and no Python (see
[known limitations](known-limitations.md#assistant-document-engines-2026-11-07)).

### Previews

The model sees a preview of the first page only if it can see images. A PDF's
preview is Typst's PNG of page 1; a PowerPoint's is officecli's SVG of slide 1,
which the runtime draws with `@napi-rs/canvas`. The canvas's SVG reader skips
`foreignObject`, where officecli puts every text box, so the runtime draws the
shapes first and then the text itself (`assistant-runtime/src/slide-text.mjs`):
it reads each box's place, paragraphs and runs from the SVG, takes 1 pt as 2 px
(officecli's scale for any slide size), wraps the lines to the box as the
slide's CSS does, and lets the canvas fall back to a system font for CJK. Text
is drawn over all shapes rather than in paint order, and a box under a
transform other than `translate`, or inside a chart's nested `<svg>`, is left
out. An SVG that is not XML gets no preview rather than a slide without its
words. Both become a JPEG of at most 768
px on the long edge and 200 KB, at a lower quality if needed and left out if it is
still too large, so a tool result stays well under the 1 MiB line limit. Word and
Excel files have no preview. The model also gets the file's id, name, type,
size, page, slide or sheet count and the issues; it never gets a file path.

Besides that first page, the host exports every slide of a PowerPoint as SVG
(`view svg --start n --end n`, one run per slide, for at most 60 slides) and
keeps the pages beside the attachment, at
`<session id>/attachments/<attachment id>.pages/<n>.svg` (the attachment takes
the id of the `outputs/<docId>/` folder it was made in): the folder is 0700, the
files 0600, and the folder is removed with the attachment. A page whose SVG is
over the 256 KiB output limit is skipped with the warning
`slide n has no preview`, and the others are kept; if the engine fails in the
middle of the export, the slides not exported get one warning,
`slides a-b have no preview`, and so do slides 61 and on of a longer deck. The
generation record, the attachment's details and the result carry `svgPages`,
the number of slide pictures kept beside the attachment (a skipped slide is not
counted, and none are when the pages cannot be copied there); a reader should
still check that the `.pages` folder exists, since
backups, restores and forks copy only the attachment file. Nothing in the app
shows these pages yet; they are the input of the page-by-page preview in the
[roadmap](assistant-roadmap.md).

### Previewing before delivering

Where the host offers Python, a pptx can be made in two steps, so the model looks
at the deck before the user is shown anything.

- `create_document({ name, format: "pptx", content, preview: true })` makes the
  file and runs all its checks and SVG pages in a new document folder, and
  delivers nothing: no attachment, no card, no generation record, and it is not
  one of the reply's three documents. The result holds a `previewId`, the slide
  count and the issues; the model also gets an overview image of the slides.
- The overview is one contact sheet that the runtime draws from the slides' SVG
  pages (with their text, as for the first-page preview above): two columns, each
  slide numbered in order, up to 24 slides (it says so when a deck has more). A
  model that cannot see images gets the issues only.
- To deliver, the model calls again: either with content and without `preview`
  (the code may have changed: this makes a new file) or with
  `fromPreview: "<previewId>"` and no content, which delivers the previewed file
  as it is, with its checks and slide pages, without running an engine. A
  preview is delivered once, in the reply that made it and in its own format;
  `fromPreview` together with `content` or `preview` is refused. The delivered
  attachment takes the preview folder's id.
- A file name may be previewed twice per reply (a preview that fails gives its
  count back). A third is refused with a message to deliver.
- When a reply ends without delivering a file it previewed, the host delivers
  that file's latest preview itself, as the document of the call that made it
  (within the reply's three documents). A reply the user stops delivers nothing:
  the user never saw the previews. Preview folders go with the others when the
  reply ends.

### Not in this round

Editing or inspecting an existing Office file (`inspect_document` and
`edit_document` in the design) is not built: the model can only make new files.
Python is used for PowerPoint decks only; Word and Excel files, and Python on
Windows, are later slices (see the [roadmap](assistant-roadmap.md)).

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
7. On macOS, where the Python route runs, ask for the same deck as the research's
   reference sample ("讲解 Transformer") and put the two side by side (see
   [the quality-gap research](superpowers/research/2026-11-08-ppt-quality-gap.md)):
   every slide should have a title, a one-sentence point, two to four zones and a
   visual that explains; the reply shows one card, made after the model's own
   preview.

Record the result here, one line per prompt, with the date, build and macOS version.

| Date        | Build | macOS | pdf | pptx | xlsx | docx | Notes |
| ----------- | ----- | ----- | --- | ---- | ---- | ---- | ----- |
| not yet run |       |       |     |      |      |      |       |

### Real-model regression: `pnpm assistant:real-eval`

The four prompts above also run without the UI, against the real gateway, on
three models. It spends real money: 4 prompts × 3 models is 12 runs of the
host. Run `pnpm assistant:prepare` once first, so the runtime and the engines
exist.

```sh
pnpm assistant:real-eval [--apex ofox.ai|ofox.io] [--account assistant-key] \
  [--models terra,luna,deepseek-v4.1-flash] [--out DIR]
```

- The key is `OFOX_API_KEY`, or else the macOS keychain item with service
  `ai.ofox.desktop.apikey` and account `--account` (default `assistant-key`; a
  tool's slug such as `codex` works too). It reaches the test through the
  child's environment only, never a command line, and the script masks it in
  everything it prints and writes.
- `--apex` picks the gateway region (`https://api.<apex>`). Each `--models`
  pattern picks the first model of `/v1/models` whose id contains it; a pattern
  that matches nothing stops the run (exit code 2) and lists the catalog.
- It runs the ignored test `assistant_host real_eval` (see
  `src-tauri/tests/assistant_host.rs`) and writes `OUT/report.md`; `OUT` is
  `.real-eval/<time>` unless `--out` says otherwise (the folder is git-ignored).
  Each case's files, their text, the rendered `.pages` and the reply are under
  `OUT/<model>/<case>/`, to open by hand.
- A case passes when the reply made exactly one file, with zero warnings, and no
  Office file's text holds a literal `\n`; every case but the pptx also takes
  exactly one `create_document` call. The pptx case may take one call per preview
  and one to deliver (or none, when the end of the reply delivers the last
  preview). Its row says which route the model took, `route` `python` for deck
  code or `deck` for a deck object (`-` for the other cases), and how many
  `previews` it made; a call that failed shows in the result's note. The report
  ends with `通过 N / (模型数 × 4)`, 12 for the default three models. The exit
  code is 0 for at most one failure and 1 for more; 2 is a usage or environment
  error.
- `--self-test` checks the script itself with no key, no network and no cargo,
  and `--help` lists the options.

The report's table is pasted into the table below, one row per case, with the
build and the tally `通过 N / (模型数 × 4)` under it. The columns are the date,
the model, the case, the route (`python`, `deck` or `-`), the previews, the
number of files, the warnings, the layout fixes applied (`fixed`), the
`create_document` calls, the seconds, the tokens (input / output) and the
result.

| 日期        | 模型 | 用例 | route | previews | 文件数 | 警告 | 自动修正 | 调用次数 |  秒 | tokens | 结果 |
| ----------- | ---- | ---- | ----- | -------: | -----: | ---: | -------: | -------: | --: | ------ | ---- |
| not yet run |      |      |       |          |        |      |          |          |     |        |      |
