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

Entitlements are chosen per executable name:

| Executable                     | Entitlements file                                | Keys                                                                                                          |
| ------------------------------ | ------------------------------------------------ | ------------------------------------------------------------------------------------------------------------- |
| `node`                         | `assistant-runtime/node-entitlements.plist`      | `allow-jit`                                                                                                   |
| `officecli`                    | `assistant-runtime/officecli-entitlements.plist` | `allow-jit`, `allow-unsigned-executable-memory`, `disable-library-validation` (all `com.apple.security.cs.*`) |
| `typst` and every other Mach-O | none                                             | hardened runtime only                                                                                         |

officecli needs `allow-jit` because the .NET runtime compiles code at run
time. The other two keys are the usual companions for .NET under the hardened
runtime: executable memory the runtime does not map as JIT, and loading native
libraries that are not signed with our Team ID.

### Spike result (2026-11-07, macOS 26, arm64)

Signed with `Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)`:

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:prepare
codesign -dv --entitlements - assistant-runtime/resources/engines/officecli/officecli
```

```text
Identifier=officecli
Format=Mach-O thin (arm64)
CodeDirectory v=20500 size=66453 flags=0x10000(runtime) hashes=2066+7 location=embedded
Timestamp=Nov 7, 2026 at 02:05:54
TeamIdentifier=R65NVCLBMY
[Key] com.apple.security.cs.allow-jit                          [Bool] true
[Key] com.apple.security.cs.allow-unsigned-executable-memory   [Bool] true
[Key] com.apple.security.cs.disable-library-validation         [Bool] true
```

typst reports `flags=0x10000(runtime)`, `TeamIdentifier=R65NVCLBMY` and no
entitlements. `codesign --verify --strict` passes for both.

```sh
APPLE_SIGNING_IDENTITY="Developer ID Application: NICE TALK PTE. LTD. (R65NVCLBMY)" pnpm assistant:smoke
```

```text
Production smoke passed (v22.23.4; Pi SDK, native canvas, PDF text and rendering, native JSONL, search server, officecli 1.0.160, typst 0.15.1)
```

The smoke test runs the signed engines from the prepared resources:
`officecli --version`, `create` and `add` of a DOCX, `typst --version` and a
PDF compile. It passed on the first attempt with all three entitlements, so
neither fallback was needed. An extra run of the signed officecli created
and edited a PPTX and an XLSX, validated both, and rendered the PPTX to HTML.

For reference, signing a copy of officecli with the hardened runtime and no
entitlements fails at start-up (`Failed to create CoreCLR, HRESULT:
0x80070008`), so `allow-jit` is required. With `allow-jit` alone, the same
operations also ran, so the other two keys are headroom for code paths this
spike did not exercise rather than something it proved necessary. They are kept
because the .NET runtime commonly needs them and a missing one only shows up
as a crash in a user's session.

Gatekeeper rejects the staged executables (`spctl`: `Unnotarized Developer ID`)
until the application is notarized. That is expected.

### Notarization (to do, by the release owner)

Not done in this spike: `APPLE_ID`, `APPLE_PASSWORD` (app-specific password)
and `APPLE_TEAM_ID` are not available on the development machine. Notarization
covers the whole application, including these engines, so it is verified by a
normal release: either through the release workflow with its secrets or locally
with `scripts/release.sh` after exporting the three variables. A dry run that
reaches `Accepted` from `notarytool`, followed by `spctl -a -vv` on the
installed application and one `pnpm assistant:smoke`-style run of the engines
from the installed bundle, closes the spike. Both executables were signed with
a secure timestamp and target macOS 12.0 or earlier (officecli `minos` 12.0,
typst 11.0), within the application's 12.0 minimum.

### If the signed engine fails

If a signed officecli is killed or reports a code-signature or library
validation error, first remove `disable-library-validation`, then add
`com.apple.security.cs.allow-dyld-environment-variables`. If neither helps,
fall back to installing officecli on first use through the existing
`install_tool` flow on macOS (download and check against the release's
`SHA256SUMS`, keep it in the application data directory); all three Office
formats stay available. Typst is unaffected because it needs no JIT.

## Windows

Nothing extra. Windows has no hardened runtime or entitlements. The engines are
bundled as `officecli.exe` and `typst.exe`, exactly as downloaded and verified
against the digests above. The Windows installer is not code-signed yet (see
the release workflow), so the engines are not either.

## Smoke test

`pnpm assistant:smoke` fails if a manifest entry for either engine is missing,
if either engine does not report its pinned version, if officecli cannot create
and edit a DOCX, or if typst cannot compile a PDF. CI runs it after
`assistant-prepare`, and a missing engine notice fails it too.
