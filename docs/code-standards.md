# Development standards

A change contains the user-visible behavior, a failing regression test that demonstrates the previous behavior, implementation, relevant documentation, and verification evidence. Keep unrelated refactors separate. Follow [repository guidelines](../AGENTS.md) and [lessons](../LESSONS.md).

## Tests and review

1. Establish RED in a temporary sandbox, then implement GREEN. Prefer assertions on results and failure behavior over implementation details.
2. Review each integration sharing the changed invariant. Model, endpoint, shared-key, session, and config changes require sibling-agent coverage.
3. Run independent review, fix actionable findings with regression tests, and re-review. Record accepted limitations explicitly; do not describe them as fixed.
4. After the final code change, run frontend typecheck, format and all unit tests; Rust fmt, `clippy --all-targets -- -D warnings`, and all tests. Record commit SHA, OS, counts, ignored tests and logs. Earlier integration checks do not replace final checks.
5. Windows-specific code must run on Windows via [the verification runner](windows-testing.md). Use the native installed agent to validate generated configuration when available; record versions and any unavailable checks.

All test writes use temp homes/config/DBs. Shared environment and settings use `test_sandbox` and the default `serial_test::serial` lock. Never use a production account, live configuration, system Trash or actual uninstall to simulate destruction. File modifications use `atomic_write`/`FileTxn`, maintain rollback and do not replace unreadable input with defaults.

## Acceptance and delivery

Check account switching and stale replies, both apex choices, empty/error/older-backend states, cancellation, repeated submission and operation event isolation. UI work includes light/dark, 900px layouts, keyboard Tab/Shift+Tab/Escape, accessible labels and dialog focus handoff. Preserve camelCase IPC and schema v11 unless a justified migration is part of the change. Visible copy covers zh/en/ja with identical key and placeholder sets.

Use Conventional Commits (`fix(scope): behavior`, `feat(scope): behavior`). PR descriptions explain trigger, old/new behavior and validation; complete [the checklist](../.github/pull_request_template.md). Do not include secrets or personal/internal paths. Commit/push/release/install only within the user's current authorization.

## Release gate

[Release](../.github/workflows/release.yml) calls the complete [CI](../.github/workflows/ci.yml) through `verify`. An independent `windows-probe` job queries the named Windows Probe workflow and requires a completed successful run for the exact release SHA in the same repository, triggered by push or workflow_dispatch. PR runs test a merge ref and cannot certify the source head SHA. Both jobs block `build`. Missing, failed, pending, other-SHA, other-workflow and fork evidence cannot pass. Run Windows Probe against that commit first, then dry-run release; dry-run still builds/signs but skips publishing. No local macOS report substitutes for the Windows gate.
