# Mac / Windows assistant integration

This local integration combines Mac snapshot
`ba953a277fde7826083f36b3cdec15698b3f2c9c` with Windows snapshot
`fce2b5bb088d7dbd49b34eaf46f90877c44fad8d`. It preserves both parents' history.
The original working trees, including their uncommitted changes and staging,
remain separate. This integration does not authorize a push, main merge,
release or installation.

## Combined behavior

The Mac task-admission boundary remains in place: ordinary questions can answer
in their first model request, while actual multi-operation work needs a visible
plan. Full Auto controls approvals and grants no new tools. Local operations
remain limited to the tool and physical home selected for the current user turn.
Saved progress, cancellation and plan bookkeeping retain their existing limits.

The Windows verification protections are retained inside each admitted task's
state bucket. The bucket contains the pending runtime snapshot, at most 32
verification targets, and independent Host-observed mutation debt. Actual
configuration effects survive cancellation, timeout, uncertain outcomes and
empty runtime snapshots. A different task or home cannot clear them. A matching
Host verification can; incomplete scopes cannot be cleared by guessing a tool
or home. Reservations and deferred guards use the same frozen bucket key.

Admission generation is checked under the local-state lock before reservations,
late snapshots, guard cleanup or verification can change that state. Ordinary
Stop retains actual effects; logout or an authority transition prevents old
callbacks from recreating or clearing replacement state. Internal native
executor tests retain the legacy unscoped bucket, while production local-tool
dispatch continues to reject unscoped calls.

Both run-control fields are retained: the scoped local task and the separately
pinned physical home. Home is checked before approval, after approval, and again
after waiting for the journal key and binding locks before effects. Windows
document preparation and its per-runtime-digest proof remain in place. No LPAC,
capability, child-process, negative-probe or failure-closed isolation check is
removed by this resolution.

## Conflict and regression coverage

The seven conflicted files cover runtime continuation, native task state,
request admission and their tests. Their resolution retains the Mac planning
opening and the Windows native Office completion helper together. Scoped
runtime tests use only currently admitted managed repairs; low-level native
tests continue to cover lifecycle operations and canonical aliases.

Two additional regressions first failed against the Mac snapshot: an incomplete
resumed repair was cleared by a guessed home check, and a different tool's check
in the same home cleared a repair. They pass with the combined verification
model. Further native cases cover real journaled effects interrupted by Stop or
timeout, unrelated tools/homes, empty snapshots, actual verification in the
original scope, and late snapshots after authority revocation and replacement.
All settings, mutations and credentials used by these tests are sandboxed.

## Verification boundary

Checks for this integration run on macOS. Earlier installed Mac 1.5.2 and real
DeepSeek acceptance established the input snapshot's behavior; they do not
certify this new combined source. The original merge checks did not install
that tree or make paid model requests. The later local Mac installation below
records its own source and preservation boundary. Native Windows verification
and the exact-commit Windows gate remain pending.

## Mac verification (2026-11-20)

On macOS 26.5.1 arm64, version 1.5.2, the resolved merge working tree passed:

| Check                                                  | Result                                         |
| ------------------------------------------------------ | ---------------------------------------------- |
| Frontend typecheck and formatting                      | Passed                                         |
| Frontend unit tests                                    | 3,403 passed, 1 skipped                        |
| Renderer production build                              | Passed; existing large-chunk advisory remains  |
| Bundled Node runtime tests                             | 1,002 passed, 12 skipped                       |
| Search tests                                           | 102 passed                                     |
| Focused planning/local verification tests              | 112 passed                                     |
| Final incomplete-scope/other-tool regressions          | 3 passed                                       |
| Rust formatting and all-target Clippy, warnings denied | Passed                                         |
| Complete Cargo tests                                   | 4,576 passed, 29 ignored                       |
| Installed Codex 0.166.0 native diagnosis regression    | Passed on both apexes with mock model gateways |

The installed-agent regression also covers the old Full Auto conversation's
Fizzy question, ordinary follow-up and scoped diagnosis. It makes no paid model
request and does not alter real user configuration. The bundled runtime remains
Node 22.23.4 with Pi SDK 1.5.0. Independent implementation and authority review
found no remaining blocking findings.

These checks ran against the resolved, not-yet-committed merge tree. Its compiled
test-only source identity correctly reports the Mac parent `ba953a27` and
`OFOX_BUILD_SOURCE_CLEAN=false`; it is not a clean-checkout Windows acceptance
receipt. Production source-file hashes were checked before and after the final
checks. The local merge commit records that source without claiming Windows
proof, a new installed application, or live model acceptance.

The local evidence includes `ofox-integration-rust-all.log`,
`ofox-integration-final-clippy.log`, `ofox-integration-runtime-all.log`,
`ofox-integration-search.log`, `ofox-integration-native-codex.log` and the
`ofox-integration-final-{typecheck,format,unit,rustfmt}.log` files. Original
working-tree files, staging and branch heads were checked against their backups;
only completed build/dependency caches and superseded acceptance packages were
removed to meet the build-space requirement.

## Later local Mac installation (2026-11-20)

The local integration worktree based on `8f585139`, including the subsequent
shared-skill discovery fix, was packaged and installed as version 1.5.2. Its
compiled source identity remains that base SHA with `OFOX_BUILD_SOURCE_CLEAN=false`;
the private package receipt records the patch and production-file hashes. This
is local installed-app evidence, not a clean-commit Windows receipt. No push,
main update, tag or release was made.

The original Developer ID, designated requirement and entitlements were
retained. All four fresh arm64 executables passed signature checks, and the
signed package's Node 22.23.4 / Pi 1.5.0, native canvas, PDF, search, OfficeCLI
1.0.160 and Typst 0.15.1 smoke checks passed without paid model requests. The
search smoke fixture was updated to acknowledge `beginSingleStep` before its
single search; production plan admission was unchanged.

The real resource library showed `academic-humanizer` and `bestpaper-writing`
under existing Agent skills with their shared user paths. The oversized native
skill remained rejected with a visible scan-limit warning. Neither writing
skill was imported, enabled or modified during this check. A cold full-profile
snapshot and one previous App were retained. The Assistant file tree, 62
sessions, 57 native histories, 561 messages and 78 attachments were preserved
exactly; user settings and resource definitions were unchanged apart from the
explicitly verified automatic price, balance and usage-health cache refreshes.
Duplicate package output was removed after acceptance.

## Metadata-first Skills installation (2026-11-20)

The later local 1.5.2 package supersedes that rejected-payload isolation build.
Native discovery now reads bounded `SKILL.md` metadata and stops at each skill
directory, following the discovery/action boundary used by CC Switch. It does
not hash attached icons or scripts before displaying a skill. Import and
explicit content comparison retain full payload and managed-artifact checks;
listing a large skill does not establish that it fits the artifact import limit.

The installed macOS arm64 App's main executable SHA-256 begins `19b95751`.
Its four executable hashes match the package receipt, and strict signature
verification, the original Developer ID, designated requirement and
entitlements were retained. Production packaging used optimized release
settings with debug assertions disabled. The signed bundle passed the Node
22.23.4 / Pi 1.5.0, canvas, PDF, mocked search, OfficeCLI 1.0.160 and Typst
0.15.1 smoke checks without paid model requests.

Native computer-use inspection of the installed resource library confirmed
`academic-humanizer`, `bestpaper-writing` and the formerly rejected
`ppt-master` under existing Agent skills, with their correct native/shared
sources. The old Claude Code and OpenCode scan-limit warnings were absent.
No skill was imported, enabled, deleted or executed during acceptance. A
separate read-only native discovery probe returned 100 entries in 106 ms using
the real Claude and shared Agents skill roots with temporary application
settings and database. This measures that probe, not full GUI startup or every
agent's real configuration.

Post-restart verification preserved the Assistant tree's 1,104 entries exactly,
including 62 sessions, 57 native histories, 561 messages and 78 attachments.
User settings, resource definitions and application paths were preserved;
only explicitly checked account-balance and usage-health cache fields
refreshed. Pricing settings were unchanged. All existing files in the three
skill source trees were unchanged.
The writing-skill tree gained only a Python bytecode cache file and its parent
directory, with a header matching the unchanged source; the creating process
was not attributed, and the cache was left in place. A fresh cold profile
snapshot and the preceding signed App remain available for rollback.

The final source checks passed TypeScript, frontend formatting, 3,406 frontend
tests (1 skipped), Rust formatting, standard all-target Clippy with warnings
denied, and 4,586 Rust tests across 23 targets (30 ignored). Rust tests reused
the shared release target with package-only test profile overrides and a
sandbox credential store; the installed production build did not use those
overrides. Independent review findings were fixed and re-reviewed.

The private receipt retains the source patch and production-file hashes for
this uncommitted worktree based on `8f585139`. No push, main merge, tag or
release was made. Windows native acceptance and the exact-commit Windows gate
remain pending.

## Incremental Mac integration (2026-11-21)

The existing integration branch retained its Mac work as local checkpoint
`ee30ab23`, then incorporated Windows candidate `8f4623f4`. The single content
conflict was in `requests.rs`. The result keeps the Mac per-turn local-tool
scope, verification debt and general task checklist alongside background
document preparation and authority-scoped runtime capability updates.

Independent review found two scheduling regressions in the incoming behavior.
A historical generated PPTX could make a plain question wait for document
isolation; a previously successful preparation could also skip recovery after
its native proof was invalidated. Both received failing barrier regressions
before their fixes. Waiting now follows only the current turn's explicit style
or verified PPTX references. Explicit document work can revalidate a recoverable
missing proof through the existing single-flight preparation, while a broken
runner stays disabled. The execution-time isolation checks remain in force.
The three portable preparation regressions passed on macOS.

The Mac planner, local verification state, metadata-first Skills discovery and
12 MiB on-demand large-page preview were checked against the Mac checkpoint
and retained. The original #1733 sample's digest was rechecked before testing.
Its historical macOS installed-app acceptance from `9c7e4803` was added to Fizzy
with the unchanged original sample digest, source scope and outstanding
Windows requirement; the card remained open.

Verification uses macOS 26.5.1 arm64, pinned Node 22.23.4 / Pi SDK 1.5.0 and
temporary test configurations. Runtime and search checks passed 1,003 and 102
tests respectively, with 12 runtime tests skipped; production smoke passed.
The initial runtime attempt exposed a local development canvas addon with an
ad-hoc signature beside the hardened signed Node. After retaining the addon
backup and signing that development dependency with the matching Team ID, the
full check passed. The installed App and Node executable were not modified.
Final frontend typecheck, formatting and 3,409 unit tests passed (1 skipped),
as did Rust formatting and standard all-target Clippy with warnings denied.
The full Rust suite passed 4,589 tests across 23 targets with 31 ignored.
The actual installed Codex 0.166.0 diagnosis / old Full Auto regression passed
on both apexes with mock model gateways. The opt-in original #1733 Host case
also passed: pages 1, 2 and 3 returned distinct bounded JPEGs, with original and
staged file digests unchanged. These last checks reused the exact test binaries
from the completed full Cargo invocation. No paid model was called.

Cargo tests reused the shared release target with package-only test overrides
(optimization 0, codegen units 256 and debug assertions enabled) and sandbox
credentials. Checks exercised the uncommitted merge tree based on `ee30ab23`;
the private receipt binds its input file hashes. The later local merge commit
records that source, without claiming clean-commit Windows evidence. No App
replacement or new installed-GUI acceptance was performed in this increment.

This receipt covers local Mac integration. The user continues Windows native
acceptance separately. The integration remains local, with the original work
and recovery snapshots preserved; no push or main merge is authorized here.
