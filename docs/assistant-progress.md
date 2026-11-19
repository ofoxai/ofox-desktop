# Assistant task progress

The assistant shows a compact activity and elapsed-time line beneath its current
reply. Multi-stage tasks can additionally report a checklist. Ordinary greetings,
short answers and translations do not need a plan. The shared `updatePlan` tool is
available to tool-capable chat models independently of local configuration;
reporting a plan grants no file, shell or MCP access and follows existing tool
budgets and run termination rules.

The checklist is initially expanded; routine operation records, execution intent
and MCP details are initially folded behind the activity line. Pending approvals,
host warnings and observed failures remain visible. A later call in the same
activity category does not prove that an earlier failed call was retried.
Completion folds the final checklist into a summary. Reported incomplete steps
stay incomplete; tool return values and a completed run are not proof that every
planned step or deliverable succeeded.

The Host saves optional per-message `taskProgress` presentation metadata through
the existing atomic session store before publishing each `task_plan` event. The
snapshot contains a fixed `startedAt` and bounded `steps`: at most twelve steps,
forty Unicode scalar values per label, and one `in_progress` step. Native history
may retime a message's `createdAt`, but does not retime the progress snapshot.
Duration uses the fixed start and existing `completedAt`; an interrupted reply
without an observed end time shows no inferred duration. Steering starts a fresh
reply snapshot. Native history merge, forks and portable backups preserve the
saved checklist without making it executable or authoritative.

Messages from older versions can omit `taskProgress`. Empty plans do not add a
historical summary. Routine local operation records remain bounded live-view
metadata; previously supported MCP history remains available. There is no database
schema migration.

Plan labels are validated before captured-credential redaction and Unicode
bounding. Canonical state values remain typed at the registered plan tool and
the Host's exact progress/outcome paths; they are not arbitrary text fields.
Backup credential checks preserve raw rejection for unrelated fields and numeric
spellings, and only exempt validated checklist-state enum values.

## Verification boundaries

Regression checks cover simple chat, folded details, terminal summaries, focus
handoff, approval animation, stale events, steering, fixed timing and portable
history. Browser checks use production React components with synthetic data at
900 px in both themes; they do not validate an installed desktop bundle. The
bundled Node 22.23.4 and Pi SDK 1.5.0 are exercised against local mock gateways for
both `ofox.ai` and `ofox.io`. No paid production-model call is needed for those
checks. The installed Mac check below covers a real model. Native Windows
planning behavior remains unverified.

## Verification record (2026-11-16)

Validated on macOS 26.5.1 arm64, in the working tree based on `c5888464`.

| Check | Result |
| --- | --- |
| TypeScript and source formatting | Passed |
| Frontend unit tests | 3,362 passed, 1 skipped |
| Bundled runtime and search tests | 816 + 102 passed, none skipped |
| Renderer production build | Passed |
| Rust formatting and all-targets Clippy with warnings denied | Passed |
| Complete Cargo tests, including integration and doctests | 4,540 passed, 27 ignored |

Independent reviews and re-reviews closed the identified focus, folded-failure,
redaction-expansion and typed-state issues. Regression cases include both apex
choices, actual Pi tool calls, private sidecar RPC, native history, fixed timing,
steering, Unicode labels and backup/restore credential guards. The ignored tests
retain their existing opt-in requirements; these results do not claim installed
desktop, paid production-model or native Windows acceptance.

## Mac handoff (2026-11-19)

The Mac owns `codex/assistant-task-progress`, branched from `main` at
`c5888464`. The runtime plan tool is in `a2aef0ae`, the Host and session
store in `b5cd31dc`, and the renderer and translations in `ecc622a8`.
On macOS 26.5.1 arm64, the handoff checks passed on that source tree:
TypeScript, frontend formatting, 607 component tests and 132 bundled-Node
tests covering planning, protocol, runtime and search. The full Rust and
frontend suite results above came from the November 16 run.

This feature can reach `main` independently of the Windows document sandbox.
After the first PR lands, the other branch must integrate the new `main` and
exercise the shared Host event and request behavior, optional frontend API
fields and matching zh/en/ja translations.

## Installed Mac check (2026-11-19)

On macOS 26.5.1 arm64, the release app built from `e781b5bb` ran two new
conversations with `openai/gpt-6.1-sol` in a temporary profile. The local
acceptance installation used a separate bundle identifier and version 1.5.2.

In the first conversation, the model updated a three-step checklist several
times, including moving a step back to in progress. After 2 min 28 s the
model API returned HTTP 400 (`invalid_request_error`): it reported that a
referenced item belonged to a different resource. The Host recorded
`assistant_model_request_failed` and retained the last checklist at 1/3.
The source of the resource mismatch remains unknown.

In a second conversation, the live checklist showed the topic confirmed,
question writing in progress and the host summary pending. Clicking Stop
during generation changed the reply to `stopped`. After opening another
conversation and returning through the history list, the summary still showed
1/3 and 20 seconds. Expanding it showed one completed and two unfinished
steps. The stored reply ID, start timestamp, labels and all three step states
matched the streaming snapshot, including the distinction between
`in_progress` and `pending`.
