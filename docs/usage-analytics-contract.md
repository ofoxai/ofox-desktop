# Usage analytics IPC contract

This document describes the version 1 query and request snapshot implementation.
Database schema 11 is unchanged. It is separate from the legacy saved-cost APIs
and from source parsing, maintenance and session analytics.

## Query and receipts

The new commands accept camelCase arguments. `UsageQueryV1` is:

```ts
type UsageQueryV1 = {
  version: 1;
  startMs: number | null;
  endMs: number;
  toolIds: string[];
  modelIds: string[];
  projectIds: string[];
  sourceIds: string[];
};
```

Times are Unix milliseconds and the interval is `[startMs, endMs)`. A null start
means all earlier history. An empty interval is valid. ID arrays are exact,
case-sensitive matches; empty arrays mean all. Normalization sorts and removes
duplicates, permits at most 256 IDs per array and 512 bytes per nonempty ID.
Overview and request queries reject nonempty `projectIds` until a proven project
join is available. They do not guess a project from a provider or a tool.

Every versioned response carries an explicit receipt:

```ts
type QueryReceipt = {
  appliedQuery: UsageQueryV1;
  queryFingerprint: string;
  snapshotVersion: string;
};
```

`appliedQuery` is the normalized query actually applied. The overview fingerprint
hashes that query. Request fingerprints hash the normalized query and normalized
request filters together. `snapshotVersion` hashes the read projection and its
pricing context. Neither value uses a wall-clock timestamp as a substitute for
a database snapshot. Each initial response reads from one SQLite transaction.
Separate overview and request calls are separate projections; they do not claim
a shared cross-command persistent snapshot.

`get_usage_capabilities()` returns `{queryVersion: 1, requestSnapshots: true,
csv: true, projectFilter: false, sources: SourceCapability[]}`. The frontend must
check this before relying on versioned filters with an older backend.

## Overview

`get_usage_overview({period, query?})` preserves `today`, `7d`, `30d`, and `all`.
When query is present it is authoritative; a 90-day or custom UI range sends
`period: "all"` and an explicit query. Query mode is a pure database read and
does not synchronize sources, fetch prices or rewrite saved costs. Without query,
the previous period planning and debounced synchronization remain available.

The response retains existing totals, series, tool/model breakdowns and
`costUsd`. It adds the receipt, `coverage`, `pricingCoverage`, `options`, `sources`,
and `estimate` in each totals/series entry. `options` contains `toolIds`, `modelIds`
and `sourceIds` from available historical detail and rollups, not only tools with
current bindings. Overview `start`, `end`, and series timestamps remain
milliseconds in both query and legacy no-query mode. Legacy request-log filters
retain their seconds contract.

Overview combines live `proxy_request_logs` and retained `usage_daily_rollups`.
It does not add `session_usage_archive`, which would count retained usage twice.
Legacy date rollups have local-date precision only. Only fully included natural
days can enter an exact query; partial boundary days are excluded and reported.
Source-specific queries exclude rollups whose source identity was not retained.
Detailed request queries exclude all rollups while reporting unavailable history.

## Coverage and measurement

```ts
type Coverage = {
  state: "complete" | "partial" | "unsupported";
  reasons: string[];
  timeBasis: "eventTime" | "legacyLocalDate" | "mixed";
  detailStartMs: number | null;
  detailEndMs: number | null;
  aggregateOnly: boolean;
  excludedRollupDays: number;
  sourceIds: string[];
};
type SourceHealth = {
  sourceId: string;
  toolId: string;
  status: "unknown" | "ok" | "partial" | "failed" | "unsupported";
  lastAttemptAtMs: number | null;
  lastSuccessAtMs: number | null;
  errorCode: string | null;
};
type SourceCapability = {
  toolId: string;
  sourceIds: string[];
  usageSupported: boolean;
  sessionAnalyticsSupported: boolean;
  projectFilterSupported: boolean;
  reason: string | null;
};
```

Unknown source health cannot certify complete coverage. Source-specific health
is read from the database setting `usage_source_health_v1`; absent sources remain
unknown. A combined synchronization finish timestamp does not prove that each
source was read successfully. Supported usage readers are Claude, Codex, Gemini
and OpenCode. Unsupported tools are explicit registry entries, including the
ChatGPT mode whose records belong to Codex. No separate ChatGPT usage is inferred.

Request rows add `observedStatusCode: number | null` and
`observedLatencyMs: number | null`. Imported placeholder success codes do not
become observed HTTP status. Proxy latency, including an observed zero, is valid;
positive native OpenCode latency is valid. Other unobserved latency is null.
These fields distinguish absent measurements from zero milliseconds.

## Current OFox base-token estimates

The policy is `current-ofox-base-v1`. The catalog is fetched anonymously from
`https://api.{resolved ofoxApex}/v1/models` with a 15-second timeout and 4 MiB
response bound. Only `ofox.ai` and `ofox.io` are accepted. A response is cached
under the apex captured before the request; it is never reinterpreted as a
response from a later selected region. Cache freshness is 24 hours per apex.

Normalized catalog snapshots are stored independently in database settings keys
`ofox_usage_pricing_catalog_v1:{apex}`. They retain four optional Decimal rates
for new input, output, cache read and cache write. Cache write selects the
5-minute field when present, otherwise the flat field. Missing, empty, negative,
unrepresentable and invalid rates remain unknown; explicit zero is distinct.
Scientific notation is accepted without rounding an unrepresentable amount to
zero. Catalog versions are stable hashes of normalized pricing content, including
apex and additional-unit rates, not fetch time. A video-unit price change updates
the version even though that unit is not estimated. Alias collisions remain ambiguous; an exact canonical ID has
priority. Damaged or unsupported cached catalogs yield unavailable estimates
with an error code while usage counts remain queryable.

```ts
type CostEstimate = {
  knownCostUsd: string | null;
  availability: "known" | "free" | "partial" | "unknown";
  unpricedModelIds: string[];
  unpricedRecordCount: number | null;
  additionalUnitsUnsupported: boolean;
};
type PricingCoverage = {
  policy: "current-ofox-base-v1";
  apex: string;
  catalogVersion: string | null;
  catalogUpdatedAtMs: number | null;
  catalogState: "missing" | "ready" | "invalid" | "unsupported";
  errorCode: string | null;
  stale: boolean;
  unpricedModelIds: string[];
  unpricedRecordCount: number | null;
  additionalUnitsUnsupported: boolean;
};
```

Known costs are exact decimal strings. Missing bucket prices yield a known
subtotal plus explicit partial/unknown coverage. Nonzero or invalid additional
units, such as video seconds, images, audio or searches, set
`additionalUnitsUnsupported`. A zero base-token estimate with such charges is
partial and must not be presented as a free model. These estimates do not cover
tiered billing or additional units. `unpricedRecordCount` is null when the
available aggregate cannot prove a count. Aggregation overflow is an explicit
error and cannot silently restart a discarded sum.

The current estimate path never rewrites historical `total_cost_usd` or the
legacy pricing table. The existing price synchronization path still updates the
legacy table and invokes its historical zero-cost backfill for legacy callers;
that saved-cost behavior is separate from versioned read-only estimates.

## Requests, detail and export

```ts
type UsageRequestFilters = {
  sourceIds?: string[];
  statusCode?: number | null;
  statusKind?: "recorded" | "unrecorded" | null;
  search?: string | null;
};
```

`get_request_logs({filters: {}, page, pageSize, query, requestFilters?,
snapshotId?})` creates a frozen snapshot when page is 0 and snapshotId is omitted.
Page size must be 1 through 100. Rows sort by `created_at DESC, request_id DESC`,
including a deterministic tie-break for records in the same second. Search is
trimmed, case-insensitive literal substring matching; `%` and `_` are not SQL
wildcards. It searches request/provider/tool IDs, models and provider names, not request bodies.
Nonempty legacy filters cannot be mixed with a versioned query.

The result includes `data`, `total`, `page`, `pageSize`, `snapshotId`, receipt,
explicit normalized `requestFilters` (arrays plus nullable fields), coverage,
pricing coverage, options, sources and summary. Summary aggregates precisely
the matching readable detail records. It does not silently reuse overview KPIs
that ignore request-specific filters or include archived rollups.

Subsequent pages reuse snapshotId and the same normalized query/filter. Query,
request-filter or apex mismatches are rejected. A changed filter or explicit
refresh starts again at page 0 without snapshotId. Background inserts cannot
shift the already frozen result. The frontend must not refresh it by default
every 30 seconds. A native source refresh is a separate operation from replacing
the displayed request snapshot.

`get_request_detail({requestId, snapshotId})` returns the frozen row or null,
including its receipt and current estimate. The receipt must match the list.
Without snapshotId, legacy detail behavior remains available.

`export_request_logs_csv({snapshotId, destination?})` exports every matching row
in that snapshot, not only the visible page. With no destination it returns a
preview `{fileName, csv, recordCount, coverage, queryFingerprint,
snapshotVersion, savedPath: null}`. After a native save dialog chooses an
absolute destination, the same command writes via `atomic_write`, returns that
savedPath and an empty csv string. Failed/expired snapshots are errors, never
silently replaced with a newer query. CSV quotes every cell, escapes quotes and
neutralizes spreadsheet formulas. It excludes request/response bodies,
credentials and error text.

Snapshots live only in memory, expire after 10 minutes and are lost on restart.
The global store retains at most four snapshots and 128 MiB of serialized rows,
evicting the oldest when necessary. An initial read is bounded to 100,000 rows
and 64 MiB of source projection and fails explicitly beyond either budget;
results are never silently truncated. CSV iterates the frozen rows in chunks of
1,000, but constructs the complete CSV in bounded memory before atomic file
write. This is not a streaming export of an unbounded database result. Export
preview and save each regenerate CSV from the same retained snapshot.

## Compatibility and verification scope

Legacy `startDate`/`endDate` filters are seconds with inclusive endpoints. New
queries are milliseconds with an exclusive end. Existing no-query commands
remain available. Current pricing estimates, coverage and snapshot receipts are
only guaranteed in versioned mode. These backend tests use temporary or memory
databases; installed agent integration and native UI behavior require separate
platform verification. No live credentials, settings or session contents are
used by the regression fixtures.
