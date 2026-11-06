# Assistant document engines

The assistant creates Office files and PDFs with two bundled command-line
engines. `pnpm assistant:prepare` downloads, verifies, signs and stages them
under `assistant-runtime/resources/engines/`, which the application ships as
`assistant-runtime/engines/` in its resources.

| Engine    | Version | Used for                         | What it is                                                            |
| --------- | ------- | -------------------------------- | --------------------------------------------------------------------- |
| officecli | 1.0.160 | Word, Excel and PowerPoint files | A self-contained, trimmed .NET 10 single-file program (not NativeAOT) |
| typst     | 0.15.1  | PDF from Typst markup            | A Rust program, no runtime and no JIT                                 |

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
therefore run them with a throwaway `HOME` and `OFFICECLI_NO_AUTO_RESIDENT=1`
and `OFFICECLI_SKIP_UPDATE=1`; the host must do the same.

## macOS signing

Notarization needs every Mach-O inside the application to be signed with the
same Developer ID, with the hardened runtime and a secure timestamp.
`signMacFiles` in `scripts/assistant-prepare.mjs` does this for everything under
`resources/` when `APPLE_SIGNING_IDENTITY` is set, and replaces upstream
signatures: officecli arrives signed by its author's team (`52JQX2HUSC`) and
typst only ad-hoc (linker-signed).

Entitlements are chosen per executable name in `scripts/assistant-entitlements.mjs`:

| Executable                          | Entitlements file                                | Keys                              |
| ----------------------------------- | ------------------------------------------------ | --------------------------------- |
| `node`                              | `assistant-runtime/node-entitlements.plist`      | `com.apple.security.cs.allow-jit` |
| `officecli`                         | `assistant-runtime/officecli-entitlements.plist` | `com.apple.security.cs.allow-jit` |
| `typst`                             | none (explicit `null`)                           | hardened runtime only             |
| any other Mach-O under `resources/` | none                                             | hardened runtime only             |

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
