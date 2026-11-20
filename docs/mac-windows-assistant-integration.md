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
certify this new combined source. This integration is not installed and makes
no additional paid model requests. Native Windows verification and the
exact-commit Windows gate remain pending.

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
