# Frozen usage parity harness

This auxiliary validation uses **Magpie bfaedf18301d02a423bd5422d22e41432d231ab8** as the reference, under its MIT license. It injects one test with Go's `-overlay`; it never changes the reference checkout or launches either application.

The four JSONL sources are entirely fictional metadata, event timestamps, identities and Token counters. They contain no conversation bodies, credentials, real user directories or API endpoints. The runner rejects other fields and requires fictional identities. Magpie's existing test sandbox isolates all native data directories and substitutes inert native clients; the injected test also blocks external HTTP. Module downloads are disabled for the actual test invocation. An installed matching Go toolchain and cached dependencies are prerequisites. Resolving an uncached toolchain with `go env GOROOT` may require Go's normal bootstrap; pass an installed `--go` executable for a strictly offline run.

```sh
python3 scripts/usage-parity/test_compare.py
python3 scripts/usage-parity/run_reference.py --magpie /path/to/pinned/magpie --output /tmp/ofox-reference-report
python3 scripts/usage-parity/compare.py --reference /tmp/ofox-reference-report/reference.json --actual /tmp/ofox-rust-report.json --output /tmp/ofox-parity-result.json
```

The output directory must be new or empty. The reference must be clean before and after the test. The report contains frozen source SHA-256 hashes; the comparison refuses missing or different input provenance. A reference report by itself establishes **the reference baseline**, not that the Rust implementation matches it.

## Independently calculated baseline

All primary sources use UTC and the same test-only model. The three consuming threads share one billing session ID. A fourth file contains only metadata and must not create an activity session.

| Metric | Expected |
|---|---:|
| Native consuming threads | 3 |
| Deduplicated calls | 8 |
| New input | 573 |
| Output | 114 |
| Cache read | 150 |
| Cache write | 27 |
| Main Tokens | 687 |
| Nearest-rank median / p90 | 193 / 395 |
| Runtime activity | 189900 ms |
| Magpie integer-seconds projection | 189 s |
| Fixed-price estimate | $0.0021765 |

`fixtures/expected.json` also freezes each thread and each date. The fixed rates per million are input **2**, output **8**, cache read **0.25**, cache write **3**. Magpie accepts those per-million rates. Ofox catalog ingestion accepts per-Token wire rates: **0.000002 / 0.000008 / 0.00000025 / 0.000003**. Using the same numeric wire values would introduce a million-fold price error.

The sources exercise paired `token_count` / `token_usage_record`, compaction metadata and its repeated top-level record, runtime counter reset, equal-sized independent calls, a shared billing ID, midnight allocation, and a pause of exactly five minutes. Additional direct `state.saw` cases cover clock rollback, a later event across midnight, the New York DST fall-back transition, and a subagent stream that must not add runtime.

Milliseconds are compared before formatting. Magpie truncates seconds after aggregating its relevant group; truncating each gap first is not equivalent. Its display-duration rounding is deliberately outside the numeric comparison. No screenshot total is treated as a target.

## Rust integration recipe

Reuse the fixtures from a **crate unit test**, where the existing crate-private source and query APIs are available. Do not add a product IPC or alter the database schema.

1. Acquire the default `#[serial_test::serial]` lock and `test_sandbox::sandbox()`. Create `Database::memory()` and copy exactly the manifest's regular JSONL files under the sandbox's Codex directory.
2. Call `session_analytics_index::refresh_sources` with the four explicit Codex `SourceSpec`s. Do not use native home discovery or live input. Freeze `projection_cache().read_on(tx)` once and check that canonical readiness is valid for the current epoch.
3. Ingest the per-Token test prices above with `PriceCatalog::ingest_on`; use UTC and `[2026-10-01T00:00:00Z, 2026-10-03T00:00:00Z)` for every consumer. Use `session_analytics_api::query`, `usage_analytics::build_query_overview`, `usage_requests::request_page` / CSV, and the cached list projection.
4. Assert that all four consumers have main Tokens **687** and the matching four buckets, that the request page contains **8** canonical events, and that none has measured HTTP status/timing or an original saved amount. Check each date, thread, quantile and fixed-price estimate against `expected.json`.
5. Reuse `session_native_activity::activity` to emit day/hour allocations in milliseconds from the same native event projection. Read the manifest's activity cases separately with their stated zones; never switch the real process timezone or read native files outside the sandbox.
6. Emit the normalized JSON shape of `reference.json` as `ofox-rust-report.json`, including the identical `sourceSha256`, `referenceCommit`, `version`, `timezone`, `tokens`, `quantiles`, `days`, `threads`, and `activityCases`. The comparator accepts stable-array reordering and equivalent decimal spellings, but rejects unknown-versus-zero, stale inputs, duplicate identities and millisecond drift. Go-only integer-seconds and call-reader diagnostics are not required in the Rust report.

The runner and reference overlay do not execute Cargo. The integration owner must run the Rust test with the shared serialized build target and record that outcome separately.

## Reference limits and intentional differences

The pinned reference **does support** the new `token_usage_record` and embedded-compaction formats used here; a generic “Magpie does not support the new format” explanation is invalid for these fixtures.

The emitted `referenceLimitations` contains two separately measured boundaries:

- Its `cxUsage.UnmarshalJSON` does not recognize the alternate `cache_read_input_tokens` field. The standalone probe returns input **100**, cache read **0**, instead of the **60 / 40** supported by Ofox's explicit alias handling. This is excluded from the common-format baseline and must be classified as a reference field-support difference.
- A missing cache-write counter stays numerically zero in Magpie's summary, though its known-field mask records the absence. Ofox must retain incomplete coverage and partial pricing; this is a coverage-policy difference, never proof of a real zero.

Historical saved costs, account charges, catalog differences, inferred timing, body/message extraction, ambiguous forks, source disappearance and unsupported formats are outside this small common-format baseline. Existing dedicated tests must continue to cover them. This harness does not validate native resource consumption, Windows, signing or notarization, and performs no paid acceptance calls.
