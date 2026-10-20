# Codex native counter fixtures

The three JSONL fixtures are adapted verbatim from Magpie `bfaedf18`,
`internal/sessions/testdata/codex-native-paginated.jsonl`,
`codex-native-compaction.jsonl`, and `codex-compaction-counter-domains.jsonl`.
They are sanitized event excerpts: no original prompts, credentials, account
metadata, personal paths, or tool output are retained. Copyright 2026 yetone;
the MIT license is preserved in `MAGPIE_LICENSE`.

The Rust counter-domain reconciler in `services/codex_native.rs` is a port of
Magpie's `codex_usage.go`, with Ofox's separate native-thread identity,
body-free message/activity projection, explicit incomplete coverage, and
conservative conflict handling. The adjacent services `MAGPIE_LICENSE` retains
the required attribution.

Native format reference: Codex `rust-v0.162.0`, `thread-store/src/types.rs`
and `protocol/src/protocol.rs`. The first session metadata is canonical;
shared session identity is not native thread identity, and inherited history
boundaries are not new child consumption.

The native reader streams large and Zstandard-compressed histories with a
64 KiB read buffer, a 32 MiB decoded-line limit, and a 2 GiB decoded-source
safety limit. Projected records remain bounded at 100,000 per source and
counter reconciliation at 4,000,000 indexed checks. Projection limits retain
explicit partial coverage and still hash the full source; hard source/read
failures never publish a truncated digest as a complete replica proof.
The small-budget reader regression generates its temporary JSONL at runtime,
so the repository does not need a giant binary or history fixture.

An optional, scrubbed first-owned-prompt title (160 characters plus a possible
ellipsis) is passed to the
existing session metadata consumer. It is deliberately skipped during native
snapshot serialization and is never included as message text in the usage
projection.

The primary-token buckets follow this fixed Magpie reference: fresh input is
inclusive input minus cache reads and cache writes; output is separate. This
differs from Codex 0.162's own CLI display helper, which subtracts cache reads
only. See `internal/sessions/codex.go` (`spent`) in the pinned reference and
[Codex's display helper](https://github.com/openai/codex/blob/rust-v0.162.0/codex-rs/protocol/src/protocol.rs#L2283-L2292).
An absent cache-write field retains the existing backward-compatible numeric
projection but keeps the event and price coverage incomplete; it is not an
observed zero. Without cache-read evidence, fresh input cannot be established,
so the call is explicitly excluded from the primary projection rather than
silently replacing unknown input with zero. Original history remains available
through the separate audit view.
