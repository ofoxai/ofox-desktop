# Native session analysis bridges

This increment connects WorkBuddy, Pi, DeepSeek Harness (`dsh`) and OpenClaw to the existing session/project analysis pipeline. It reuses their native usage readers; it does not introduce another usage collector, rewrite historical ledger ownership, or change SQL schema v11 or `UsageQueryV1`.

The capability registry explicitly advertises eight analysis bridges: Claude Code, Codex, Gemini CLI, OpenCode and these four additions. Adding another usage reader does not automatically enable its analysis bridge. Hermes remains a separate [lifetime statistics](usage-hermes-lifetime.md) source, without inferred event-time session/project metrics.

## Provider contracts

The four providers expose a complete-roster `stamp(roots)` and a read-only `read(database, roots)` returning body-free session descriptors, native usage facts, effective message events and coverage reasons. Discovery combines all roots for a provider before reading; Pi ancestry cannot be validated by parsing isolated files independently. See [discovery](../src-tauri/src/services/session_analytics_discovery.rs), [shared evidence helpers](../src-tauri/src/services/session_analytics_native.rs) and [the cache coordinator](../src-tauri/src/services/session_analytics_index.rs).

| Provider                                                              | Analysis projection                                                                                                                                                                                                         | Message/activity projection                                                                                                                                                                      | Preserved limits                                                                                                                                                                                                                                                                    |
| --------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| [WorkBuddy](../src-tauri/src/services/session_analytics_workbuddy.rs) | Native SID/message identities, recorded four token buckets and project metadata from the resolved projects roster. A verified sidecar supplies a project only when native project evidence is absent and not contradictory. | Visible user/assistant messages from the same native reader, with native timestamps.                                                                                                             | Missing identity/time/usage stays partial. Conflicting replicas fail the provider read; equal token totals alone do not prove identical conversations. A project conflict can conservatively mark an otherwise priced amount partial.                                               |
| [Pi](../src-tauri/src/services/session_analytics_pi.rs)               | All-branch paid usage uses the complete controlled roster and existing native fork/ancestor verification. Verified copied prefixes belong to the ancestor, not both sessions.                                               | Only the current native branch's visible user/assistant messages; verified inherited prefix messages are excluded from the child. A fork remains a separate conversation, not a subagent rollup. | Missing ancestors, uncertain fork boundaries, unsupported headers/schemas or conflicting native replicas do not establish ownership/activity. Reuse the [schema-3 reader limits](usage-pi-reader.md).                                                                               |
| [dsh](../src-tauri/src/services/session_analytics_dsh.rs)             | Native SID, project metadata and proven own usage use durable generation ordering, seed boundaries and previously acknowledged event evidence.                                                                              | Unavailable: `messages_supported=false` and an explicit reason. Usage events are not substituted for conversational messages.                                                                    | An older generation cannot replace a removed newer generation. Unknown seed ownership stays excluded; conflicting projects stay unparsed. Invalid parent graphs, including a delegated self-parent, do not establish a main session. See [native v0 coverage](usage-dsh-reader.md). |
| [OpenClaw](../src-tauri/src/services/session_analytics_openclaw.rs)   | Agent-scoped session/event identities. Native SQLite primary sessions take precedence over legacy JSONL, retaining the reader's acknowledged usage and dependency evidence.                                                 | Unavailable: `messages_supported=false` and an explicit reason.                                                                                                                                  | A remembered primary disappearing must not revive stale legacy events. Unknown archive/fork boundaries remain partial. Identical bare SIDs in different agents are not one session. See [schema-24 coverage](usage-openclaw-reader.md).                                             |

dsh/OpenClaw require the native usage sync's current source/roster receipt and event acknowledgement before publishing corresponding analysis facts. The bridge never grants an acknowledgement or repairs the usage manifest to make a projection pass. Their source changes must first pass native sync during the refresh round.

## Truthful unknowns and pricing

- Main Tokens are new input plus output. Cache reads and writes remain separate; missing HTTP status, duration or first-token timing is not inferred from token/output presence.
- A recorded zero, an absent metric and a read failure are different outcomes. Reliable known subsets can be displayed with coverage reasons; unavailable model/source attribution remains unavailable rather than zero.
- Message/activity metrics use valid projected messages and timestamps. dsh/OpenClaw do not acquire activity, hourly heatmap or message-distribution samples merely because they have usage facts. Activity is observed conversation activity, not human working time.
- Project metadata missing, explicitly unknown and conflicting/unparsed remain distinct. Project association is not guessed from a model, a shared bare SID or a nearby directory.
- Amounts use the current catalog for the resolved OFox apex. They are estimates, not account deductions or reconstructed historical prices. Unknown prices display unknown; a verified zero price can display zero; partially priced usage retains the known amount and partial coverage.
- A missing source preserves the last accepted history with unavailable source/navigation evidence. A failed read retains the earlier accepted projection and reports failure. Neither case publishes an empty successful replacement or re-observes lower-priority history to fill the gap.

## Snapshot, deduplication and cache

Each provider's accepted projection is one roster cache envelope. Publication verifies the native roster/dependencies before and after reading, and uses the existing compare-and-swap boundary with the current ledger epoch and provider-manifest revision. WorkBuddy includes present **and absent** sidecars; Pi includes all configured/observed roots; dsh includes generations; OpenClaw includes database/WAL and primary-authority dependencies. Invalid roots, unsupported aliases, read failures and source changes fail closed under the corresponding reader's contract.

Native duplicate identity and content evidence are checked before publishing. WorkBuddy includes transient message-content hashes; Pi reuses native entry digests and verified fork boundaries; dsh/OpenClaw retain their native generation/primary and event-conflict rules. The cache retains descriptors, facts and effective message metadata, not transcript bodies or private message hashes. Parsing is bounded; WorkBuddy message accumulation and Pi's second effective-message projection enforce a 100k entry/message bound even when no paid usage is present or the first reader stops at an unsupported header.

Queries read the accepted cache rows, effective health and current prices inside one SQLite read transaction. KPI, trend, distribution, heatmap and ranking calculations consume that same accepted input. This is a consistent database observation of accepted provider projections, not a claim that all external files were captured at one instant.

The body-free decoded projection is held in one `Arc` cache slot for at most ten minutes, with an estimated 128 MiB retention budget including native display proofs. Each warm lookup still validates the current raw rows and health. Pricing and filters are recalculated; they are not hidden behind a cached estimate. Existing in-flight `Arc`s may outlive eviction, so the slot budget is not a global concurrent-request memory limit. Frozen ranking snapshots keep their existing separate lifetime/budget.

## Session-list amount mapping

[The listing integration](../src-tauri/src/session_manager/usage.rs) performs one legacy aggregation and one body-free cache batch in the listing transaction, then captures the current catalog. It does not issue a usage query or parse a transcript for every row.

For these four providers, a listing amount requires the descriptor's exact `manager_key` derived from provider, native session identity and source path, a matching provider, available source evidence, and current roster/manifest/epoch checks. It sums that descriptor's **own** native facts. A child does not inherit a parent's total; an OpenClaw bare SID does not join another agent's records. Missing association or a changed source produces an explicit unavailable display, not a legacy zero fallback.

The optional display receipt retains `basis=currentOfoxEstimate` and `scope=nativeThread`. Here the latter means the proven native conversation's own recorded usage; it does not promise that each provider has Codex's thread format. Partial native coverage keeps the estimate partial, including when the known priced portion is zero.

OpenClaw's current session manager discovers legacy JSONL sessions. Only a verified legacy descriptor can supply that exact manager key; a primary SQLite-only session remains available to analysis without inventing a JSONL reading/navigation target. This bridge does not add native reading, export, deletion or resume capabilities to providers that lack them.

## Verification and performance record

Run from the repository root. Serialize Rust builds, reuse the existing target cache and check available disk before a release build; below 20 GiB clean only confirmed completed build output. These commands create sandbox fixtures and do not modify live agent history or configurations.

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib analytics_
cargo test --manifest-path src-tauri/Cargo.toml --lib source_refresh_six_hundred_mixed_native_bridges_stay_warm_without_body_reads
cargo test --manifest-path src-tauri/Cargo.toml --lib native_session_
cargo test --manifest-path src-tauri/Cargo.toml --lib workbuddy_analytics_messages_without_usage_are_bounded_before_publication
```

The mixed 600-session fixture has 150 sessions per new provider. It verifies projection/usage coverage and zero second-round native body reads for both periodic and manual warm refreshes. It does not measure the complete release query's latency or prove real historical completeness.

Run the opt-in performance check separately after the final source state is frozen:

```sh
df -h .
git rev-parse HEAD
rustc --version
cargo test --release --manifest-path src-tauri/Cargo.toml --lib services::session_analytics::tests::session_analytics_release_four_native_bridges_query_p95 -- --ignored --exact --nocapture --test-threads=1
```

The benchmark uses synthetic body-free accepted cache data: 10k sessions, 50k facts, 50k message events, four providers, 100 cache envelopes and a 1460-day range. After three warmups it records 30 runs, p50 and p95; the warm API target is p95 < 300 ms. It covers cache/catalog/health reads, deduplication, aggregation and frozen-cohort retention. It excludes cold parsing, native source scans, real history completeness, native IPC/serialization and UI/compositor work. Its synthetic dsh/OpenClaw messages exercise aggregation cost; they do not claim those native providers supply message/activity metrics.

Fill this table from the final verification logs; an unfilled cell is not a passing result. Attach raw logs/samples to the final delivery record rather than committing machine-specific paths.

| Evidence                                                         | Final result                                                        |
| ---------------------------------------------------------------- | ------------------------------------------------------------------- |
| Final source SHA; clean/dirty state while measuring              | Pending final run                                                   |
| OS/architecture; Rust version; relevant installed agent versions | Pending final run                                                   |
| Mixed 600-session warm-read regression                           | Pending final run                                                   |
| Four-provider release API p50/p95; raw sample/log reference      | Pending final run                                                   |
| Final frontend and Rust gates; independent review reference      | Pending final run                                                   |
| macOS installed application and window/UI evidence               | Pending separate acceptance                                         |
| Actual agent consumption/connection behavior by exact version    | Not established by bridge fixtures; record separate native evidence |
| Windows native execution on the same source SHA                  | Pending Windows acceptance                                          |

The required final gates remain [the repository checks](code-standards.md) and [Windows testing](windows-testing.md). Neither a macOS build nor this synthetic benchmark establishes Windows acceptance, real agent consumption of library configurations, live MCP connectivity, or exhaustive historical billing correctness.
