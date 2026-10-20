# WorkBuddy native usage reader

Implementation and acceptance notes for the WorkBuddy source. The shared
[query and pricing contract](usage-analytics-contract.md) remains authoritative.

## Evidence and scope

The local installed application reports **WorkBuddy 5.7.7**. Read-only inspection
of its packaged application code confirms `WORKBUDDY_CONFIG_DIR` as the native
configuration-directory override. Its resolver applies `.trim()` to this environment
value and ignores a blank result, followed by the product's data-folder name.
The inspected product specifies `.workbuddy`. Native resolver behavior was read
from `main/index.js` in the installed application archive; no application code
was executed, no AI request was made, and no user configuration was changed.

A supplied sanitized structural audit sampled 11 usage entries: all had one
request, millisecond timestamps and distinct message IDs. It observed
`cached_tokens` in input details and `reasoning_tokens` in output details. It
sampled no subagent files and does not establish semantic billing accuracy or
global message-ID uniqueness. No transcript text, identifiers, paths or
credentials from that audit are included in this repository.

The field layout was compared with Magpie commit `bfaedf18`, specifically
`internal/sessions/workbuddy.go` and `calls_workbuddy.go`. Magpie is a layout
reference. Its current-route attribution, consecutive-only reply replacement
and rejection of every prompt beginning with `<` are not accepted assumptions.
Additional installed versions and Windows native behavior remain unverified.

## Native facts

Sources are JSONL files below the resolved directory's `projects/`, including
nested files. Only absolute configuration-directory overrides are supported;
relative overrides are rejected rather than interpreted against the desktop
process working directory. Symbolic-link sources are not traversed. Historical `.meta.json` sidecars can contribute metadata, not
usage identities or billing facts. Reader overrides do not change the existing
WorkBuddy binding writer, whose `models.json` path currently uses the default
home directory. That reader/writer discrepancy is explicit until a separate
binding change is tested.

Native event identity is `(workbuddy, sessionId, providerData.messageId)`. The
current evidence supports reply identity within a native session; it does not
justify collapsing the same message ID across different sessions. A rename or
move of a physical file must not manufacture a new event. A later revision of
an event replaces its tokens, model and event time. Replicas of the same native
event are counted once; conflicting physical replicas must not select an
arbitrary winner by directory iteration order.

Modern `providerData.usage.inputTokens` includes cache reads and cache writes:

```text
new input = inputTokens - cache read - cache write
main Tokens = new input + outputTokens
```

Input details may be one object or an array of objects. Modern fields take
precedence by presence, including explicit zero; legacy nonzero values must not
replace a modern zero. `outputTokens` already includes reasoning, so reasoning
is not added again. Negative, fractional, overflowing or inconsistent cache
counts cannot be normalized as plausible zero values. Multi-request aggregate
usage is not assumed to be one native event.

Event timestamps retain native milliseconds. Missing timestamps remain absent;
the reader never substitutes scan time or a file's modification time as event
time. Missing native identities, missing billed-model evidence, incomplete
JSONL and unsupported aggregate usage carry explicit partial/unsupported
reasons. A requested route is not substituted for an unknown billed model.
Current model/vendor/endpoint configuration is never evidence of who served a
historical request.

The legacy ledger stores event time in seconds under schema 11. Normalized
source facts retain the native millisecond value. Exact subsecond filtering of
legacy ledger rows requires a shared persisted event-time projection; this
source does not pretend that truncating to seconds preserves that precision.

## Persistence and estimates

Only normalized token facts, native identities, structured reasons and source
evidence belong in the usage ledger/cache. Transcript bodies, function-call
arguments, reasoning text and credentials are not persisted in usage evidence.
Session titles are bounded metadata derived during a metadata scan; conversation
output is produced on demand. Transcript bodies are never stored in the usage
or metadata cache.
Unknown native parent relations remain unknown rather than inferred from a
`subagents/` directory name.

Usage scans allow at most 10,000 JSONL sources, 64 MiB per source,
128 MiB per changed roster and 100,000 normalized events; an exceeded budget is
an explicit failure. A completely unchanged roster skips body reads. A changed
roster rereads its bounded sources to detect conflicting physical replicas.
Session metadata currently reads each source on listing; this reader does not
claim the separate session-analytics index's warm-cache behavior.

Facts and checkpoints must commit with their source proof in one transaction.
A failed parse or changed source must preserve the previous successful facts,
checkpoint and proof. Incomplete JSONL cannot acknowledge an unfinished event.
An exact positive identity remains meaningful after detail rows are pruned;
absence of a row alone is not proof that a native event was never counted.
Incomplete identity/enumeration proof stays partial; a later full read does not
automatically restore its negative-evidence completeness. A partial model or
JSONL prefix is recorded in normalized facts/source evidence and does not itself
block unrelated maintenance. Actual file, validation, or transaction failures
remain errors.

WorkBuddy has not supplied an original dollar bill in the supported usage
shape. Imported ledger amounts retain the compatible zero placeholder; the UI
must not call that value a WorkBuddy bill. Current estimates use the shared
OFox `PriceCatalog` and its four normalized token buckets, including scientific
decimal rates, coverage and additional-unit limitations. No saved amount is
rewritten by a read-only estimate.

## Verification record

Regression fixtures use temporary files and memory databases. Tests changing
environment or settings use the default `serial_test::serial` lock and the
shared test sandbox. Native resolver metadata and the supplied sanitized audit
are structural evidence only; no AI call or live configuration/session write
was made.

The executable `workbuddy_` filter passed **72 tests**, including existing
binding/configuration siblings. The wider `usage_` filter passed **264 tests**
before the final directory-type guard; the final WorkBuddy run covers both
usage and metadata callers of that guard. New behavior regressions were run
failing first, with separate controls for native trim, invalid data and late
checkpoint failure. The late-checkpoint control checks every value in the
ledger, checkpoints, source cache and provider manifest before retrying.

Rust formatting, documentation formatting and diff whitespace checks passed.
Independent parser/provider and transaction/proof reviews found no unresolved
confirmed P1/P2 issue in this slice. `cargo clippy --lib -- -D warnings` remains
**not passed** at this savepoint: eleven shared normalized/cache items await the
session-analytics production consumer, and two existing shared-source style
lints await their owner. No lint suppression or interface removal was added.
The integrated all-targets/full-test gates, native UI and same-SHA Windows
verification remain separate integration work; this macOS synthetic result is
not Windows validation.
