# Manufacturer API price fallback

Local usage estimates first use a usable price from the selected Ofox apex.
When the model is absent, disabled, retired, or lacks a required token rate,
the estimate falls back to that model manufacturer's standard API list price.
A model uses one tariff at a time; missing Ofox buckets are not mixed with
manufacturer buckets. Explicit zero/free Ofox rates remain authoritative.

The fallback is shared by overview, requests, session analytics, and native
session display through `PriceCatalog`. It never changes an agent's runtime
configuration, credentials, saved request amounts, or account balance. Both
Ofox regions use the same independent manufacturer catalog.

## Catalog and matching

Like magpie, manufacturer prices are obtained from the first-party provider
entries in [models.dev](https://models.dev/api.json). Resellers, cloud-hosted
variants, and subscription/coding-plan entries are excluded. The accepted
manufacturers are explicitly listed in `official_pricing.rs`.

The bundled 2026-10-21 snapshot contains 273 models across 15 manufacturers.
The Anthropic, OpenAI, and Google entries were checked against their official
pricing references:

- [Anthropic API pricing](https://platform.claude.com/docs/en/about-claude/pricing)
- [OpenAI API pricing](https://developers.openai.com/api/docs/pricing)
- [Google Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing)

The snapshot consists only of public model identifiers, canonical identifiers,
and numeric list prices. It does not depend on a user's magpie or OpenCode
installation. `models.dev` is a fixed external price-data endpoint, independent
of the region-aware Ofox API endpoints.

Startup and scheduled maintenance refresh a catalog older than 24 hours;
manual usage refresh also attempts an update. Downloads are bounded to 16 MiB
and 15 seconds; failed or invalid responses retain the last verified prices.
The bundled snapshot supports first launch and offline use. Removed manufacturer
entries retain their last known list prices for historical estimates. A failed
Ofox request does not prevent the independent manufacturer refresh.

Matching accepts exact published identifiers, publisher-declared canonical
identifiers, and dot/dash spelling differences, including a recognized
manufacturer prefix. Ambiguous aliases, unknown models, and custom reseller
suffixes are not guessed. Missing rates remain unknown, never zero.

## Estimates, provenance, and limits

Amounts remain USD base-token estimates, not historical invoices. Input,
output, cache reads, and cache writes are priced separately. Supported
context-length tiers apply to an individual request's total input; a historical
aggregate that cannot establish each request's tier stays partial. One-hour
Anthropic cache writes use the corresponding official multiplier. Additional
units such as audio, tools, cache storage, and service tiers are not inferred.

`officialModelIds` accompanies per-request estimates and aggregated pricing
coverage. Coverage also identifies the manufacturer catalog version and update
time. All price data participates in the existing snapshot receipt, and frozen
pages, details, and CSV retain their original prices and provenance when a
catalog refreshes. CSV appends `estimatePriceSource` (`ofox`, `manufacturer`,
or empty when no amount is known).

The UI gives a compact manufacturer-price fallback count and identifies the
price basis in request details. A genuinely unknown model continues to appear
as unpriced. A cached list price may differ from a current or negotiated bill;
the catalog update timestamp is retained rather than claiming a successful
network refresh when a feed is unavailable.

## Validation

Validated on macOS 26.5.1 arm64 against base commit
`d3e1ad8a70131b8efd3aeca9f87d1645f1e893f4` plus the uncommitted working changes.
The regression suite uses in-memory databases and temporary sandboxes, covering
both Ofox apex choices, missing/offline catalogs, disabled models, free Ofox
rates, aliases, context tiers, hour-long cache writes, invalid refreshes,
clock corrections, and frozen request/detail/CSV provenance. Independent review
and re-review completed after fixing the reported issues.

Typecheck, formatting, all-target Rust Clippy, and all Rust tests passed
(3,384 passed, 13 ignored). The full frontend run
passed 1,445 tests; one unchanged native-acceptance Gemini parser test fails in
the installed yargs package (`require is not defined in ES module scope`).
The 27 focused pricing UI tests passed. Windows execution is not validated by
these macOS results. No public release is created by this change.

The signed local 1.3.7 app was installed with a recoverable backup and checked
through computer use. Its all-history requests page displayed 14 manufacturer-
priced models and one remaining unknown model, with the compact source hint and
without the old rebuilding/error panels. The binary SHA-256 is
`fa7670faae516eba0a391bf8e68e808cc0b9709e22eef2bdfc1bcfebe4c497fa`.

Verification logs: `/tmp/ofox-official-fallback-unit-complete.log`,
`/tmp/ofox-fallback-final-focused.log`,
`/tmp/ofox-official-fallback-clippy-final.log`, and
`/tmp/ofox-official-fallback-cargo-test-final.log`.
