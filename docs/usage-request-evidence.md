# Request detail evidence

The request page is a local usage-management view. A readable token record is
not evidence of a successful HTTP response. This audit concerns the existing
supported readers and their persisted projections, not every possible version
or field an external agent might write. It adds no network interception,
provider routing or database migration.

Existing evidence records name Claude Code 2.1.301, Codex 0.162.0, Gemini CLI
0.64.0 and OpenCode 1.19.2 in the [local implementation
record](implementation-progress-2026-10-16.md). WorkBuddy 5.7.7 has a packaged-code
and sanitized structural audit; Pi 0.84.4/schema 3 has a temporary pure native
API roundtrip, dsh 0.1.2-rc.1 has a synthetic native persistence roundtrip, and
OpenClaw 2026.10.3/schema 24 has a synthetic installed-native SQLite roundtrip.
Those existing version observations are not repeated live-agent validation by
this display-only change. Windows, other native versions and actual provider
billing semantics remain unverified within the cited source contracts.

## Source fields and presentation

| Source ID                              | Local evidence used by the current reader                                              | HTTP status in the versioned request view                        | Elapsed time                                                                              | First generated token                                                                          |
| -------------------------------------- | -------------------------------------------------------------------------------------- | ---------------------------------------------------------------- | ----------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `session_log`                          | Claude assistant usage, model, message identity and event timestamp                    | Not provided; legacy stored `200` is a compatibility placeholder | Not provided                                                                              | Not provided                                                                                   |
| `codex_session`                        | Codex native token-count events, model, session identity and event timestamp           | Not provided; legacy stored `200` is a compatibility placeholder | Not provided                                                                              | Not provided                                                                                   |
| `gemini_session`                       | Gemini reply token buckets, model, message identity and event timestamp                | Not provided; legacy stored `200` is a compatibility placeholder | Not provided                                                                              | Not provided                                                                                   |
| `opencode_session`                     | OpenCode assistant token buckets, model, native identity and message timestamps        | Not provided; legacy stored `200` is a compatibility placeholder | Positive native `completed - created` message interval; not proof of HTTP round-trip time | Not provided                                                                                   |
| `workbuddy_session`                    | WorkBuddy `providerData.usage`, model, native identity and event timestamp             | Not provided; stored `0` is a compatibility placeholder          | Not provided                                                                              | Not provided                                                                                   |
| `pi_session`                           | Pi native assistant usage, model, entry identity and event timestamp                   | Not provided; stored `0` is a compatibility placeholder          | Not provided                                                                              | Not provided                                                                                   |
| `dsh_session`                          | dsh v0 native model-assistant usage, model, native identity and event timestamp        | Not provided; stored `0` is a compatibility placeholder          | Not provided                                                                              | Not provided                                                                                   |
| `openclaw_session`                     | OpenClaw schema-24 assistant token buckets, model, native identity and event timestamp | Not provided; stored `0` is a compatibility placeholder          | Not provided                                                                              | Not provided                                                                                   |
| `proxy` (including legacy null source) | Existing historical request rows                                                       | Only recorded codes from 100 through 599                         | Recorded latency, including a real zero                                                   | Unavailable: legacy `first_token_ms` times the first SSE event, which may be a lifecycle event |

The `proxy` compatibility row describes existing retained history. This work does
not add a proxy, gateway, new request collector or background HTTP measurement.
Hermes lifetime counters do not create request rows and do not enter this table.
ChatGPT's Codex mode shares the Codex source rather than manufacturing another
source or duplicate calls.

The source IDs above are technical identifiers. The main row uses translated
source names and source-specific local-history help. The raw ID belongs in the
collapsed technical details. Missing status has no success/failure styling;
actual measured 200 and 429 remain distinguishable.

## Missing data, failures and zero

- The existing `observedStatusCode`, `observedLatencyMs` and
  `observedFirstTokenMs` fields are authoritative for display. Explicit null never falls back to compatibility columns. For older retained request records with no observed fields, only the legacy proxy source (including its null-source compatibility form) can use valid HTTP status and latency; local imports and unknown sources cannot. The old first-event column never supplies first-generated-token timing.
- No measured HTTP status means the original record has not supplied a usable
  status measurement. It does not mean token import failed. Output text or
  nonzero tokens cannot manufacture HTTP 200.
- A source read failure is a separate `SourceHealth.status = failed` or partial
  coverage result. Existing successful token facts may remain readable after a
  later failed refresh. A failed refresh does not change those records into
  failed HTTP requests or zero tokens.
- Failure rate uses only observed status samples. Zero samples yield null;
  one measured 200 yields 0%, and one measured 429 yields 100%. Unmeasured
  imports never dilute that denominator.
- Valid token facts remain in the request total and trend even without a status.
  Selecting the no-measured-status filter keeps their four token buckets. Main
  Tokens remain new input plus output; caches are counted separately.
- A compatible saved cost of `0` is not evidence of a free request. The shared
  current OFox price estimate distinguishes verified free/zero, unknown prices
  and partially priced subtotals. It does not claim an authenticated account
  charge or rewrite saved amounts.

The supported local history does not retain the actual HTTP response status,
network timing or first generated-token timestamp for these imported facts.
They cannot be recovered from token totals, assistant text, file modification
time or the scan time. OpenCode's existing native message interval is the one
bounded timing exception; it does not establish first-token timing. A future
source that supplies stronger evidence needs a source-specific contract and
regression tests before it can produce observed measurements.

## Code and verification

The existing provenance normalization is in
[`usage_data.rs`](../src-tauri/src/services/usage_data.rs); request snapshots,
detail, sample aggregates and CSV share
[`usage_requests.rs`](../src-tauri/src/services/usage_requests.rs).
See the [query contract](usage-analytics-contract.md),
[agent wiring](agent-wiring.md), and source-specific
[WorkBuddy](usage-workbuddy-reader.md), [Pi](usage-pi-reader.md),
[dsh](usage-dsh-reader.md) and [OpenClaw](usage-openclaw-reader.md) boundaries.

The new backend guards cover all eight local source siblings with nonzero
tokens, legacy 200/0 and a deliberately untrusted local 429; they verify null
observed status and failure rate, preserved token/cache totals, detail/CSV
inclusion and the existing OpenCode timing exception. A second guard mixes
measured 200/429, an unmeasured proxy record and an imported record, checking
status filters and sample denominators. These guard existing correct backend
behavior; they are not claimed as failing tests for a backend defect. The
user-visible regression is demonstrated and repaired in frontend tests.

Targeted backend command (serialized with other Rust builds):

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib services::usage_requests::tests:: -- --nocapture
```

On macOS arm64 this targeted run passed 12 tests, with no failures or ignored
tests. It includes both new behavioral guards and the existing snapshot,
filtering, timezone, first-SSE-event and CSV controls. Formatting checks cover
the added tests and this document; final repository gates are recorded
separately by the integration pass.

Native Windows behavior remains a separate acceptance step. No live agent
configuration, session files or tools are modified by these memory-database
guards.

## Display regression and acceptance

The list uses friendly source names with local-history help, omits absent status
without a failure style, and preserves its two-line row height. Details show a
specific original-record status explanation, group missing timing, and retain
raw source/request IDs and historical saved amounts under technical details.
The short no-status filter note does not imply success; selecting it displays
the full explanation that valid Tokens still count. No valid status samples
shows “No available samples”; real zero timings and zero estimates stay zero.

The initial frontend regression run failed 17 cases (one existing-compatible
case passed). Final targeted checks passed 42 tests. Full frontend gates passed
134 files / 1032 tests, typecheck and format. Full Rust gates passed 20 suites /
3012 tests with 11 defined ignores, rustfmt and all-targets Clippy with warnings
denied. The independent reviewer caught a clipped filter explanation and
English/Japanese wording that could imply a known outcome; both were fixed and
re-reviewed successfully.

Labeled synthetic browser fixtures at 900px cover light/dark, zh/en/ja, real
200/429, missing status, zero/unknown/partial prices, long names and keyboard
menu/dialog/disclosure focus. A 2448px model label is bounded to 456px without
horizontal page overflow or increasing the 56.5px row height. Screenshots and
logs are delivery artifacts, not source fixtures containing user data. The
following Tools UI increment shares the eventual desktop build; native
installation and Windows results are not inferred from these browser checks.
