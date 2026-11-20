# Assistant task progress

Ordinary greetings, short answers and translations use the regular streamed
reply. While waiting for text, the assistant can show a single status line. A
single search can show its activity without opening the task region. The waiting
line disappears when the reply text arrives. Multi-stage execution starts with a
checklist with elapsed time. The shared `updatePlan` tool is available to
tool-capable chat models independently of local configuration; reporting a plan
grants no file, shell or MCP access and follows existing tool budgets and run
termination rules.

Every ordinary tool-capable turn begins with task preparation. The first request
offers `updatePlan` and `beginSingleStep`, while allowing a plain-text answer in
that same request. Simple questions therefore need no separate classification
request. The model uses the current request and the actual enabled capability
summary to choose a meaningful multi-stage plan or exactly one operation; there
is no prompt-keyword classifier. A valid plan has at least two steps and some
unfinished work. It opens the tools already admitted for this task. Single-step
preparation exposes those same tools for one operation without opening a visible
checklist. Metadata and discovery calls also use that one-operation allowance.
Further operations require a visible plan; repeating preparation or clearing a
plan cannot replenish the allowance. The operation slot is claimed synchronously
before any async tool work. Each provider response keeps its original admission
mode, so preparing and executing in the same response cannot bypass the gate.
If an additional unplanned operation is actually attempted, the next normal
request names `updatePlan` to promote the task automatically. Merely finishing
one operation still permits a direct answer. After the valid plan is accepted,
the selected reasoning mode and admitted execution tools are restored so the
blocked operation can be retried.
Forced planning is limited to two logical model requests per input. Rejected
plans stop before another request once that allowance is exhausted. MCP tools
share this rule and keep their existing permissions.
Planning and single-step preparation use the existing shared tool, time and cost
budgets. They do not create extra execution slots beyond the configured budget.
An exhausted shared tool budget prevents further model requests, including after
steering; steering resets the planning allowance alone.

An explicit PPTX picker selection is already known to need multiple stages. It
offers only `updatePlan` with the protocol's named-tool choice at preparation.
DeepSeek and Anthropic use their compatible non-thinking mode for that forced
request, then resume the chosen reasoning setting for execution. Ordinary chat
can answer directly; an actual second unplanned operation uses that same named
planning choice for automatic promotion. Tool-free models, summaries,
compaction and text-only closeout skip execution preparation. A new steered reply
gets a new task decision without carrying a previous plan or single-step slot.

The checklist is initially expanded; routine operation records, execution intent
and MCP details are initially folded behind the activity line. An observed
sequence of calls or a tracked MCP step can also expose execution details when
there is no checklist. Pending approvals, host warnings and observed failures
remain visible. A later call in the same activity category does not prove that an
earlier failed call was retried.
Completion folds the final checklist into a summary. Reported incomplete steps
stay incomplete; tool return values and a completed run are not proof that every
planned step or deliverable succeeded. A PPTX checklist ends with delivery. With
an explicit PPTX picker selection or a Host-frozen slide plan, the runtime keeps
this step pending until `create_document` confirms a saved PPTX attachment for
the current reply. The model can then update the same checklist. A preview can
support review before delivery. If the Host saves a preview during run cleanup,
the attachment can be opened and the checklist retains the last reported steps.

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

## Follow-up on the rejected model request

The retained first-run records contain seven HTTP 200 chat requests with
successful `updatePlan` results, followed by a rejected eighth request on
the Chat Completions path. The seven native tool-call IDs match their
results. The saved conversation contains five encrypted reasoning items
with gateway routing suffixes. The archive retains gateway correlation
IDs and native messages, while the original serialized request and the
upstream resource chosen for each attempt remain unavailable.

The bundled Pi regression uses synthetic reasoning items and seven plan
updates on both apexes. It checks exact reasoning replay and tool-call
pairing through request eight, with both a successful final response and
an injected HTTP 400. A Host integration test checks that the rejected
request is not retried and that a new Host instance reads the same final
checklist, including the in-progress and pending steps.

The follow-up ran on macOS 26.5.1 arm64 from `bddb63f9` with these test
changes. TypeScript, formatting, all-targets Clippy and the renderer build
passed. The frontend suite passed 3,362 tests with one skipped. Cargo passed
4,541 tests with 27 ignored. The bundled Node runtime passed 821 tests and
the search server passed 102 tests, with none skipped.

Determining the cause of the installed-app rejection requires comparing
the actual provider, upstream resource and fallback attempts for the
seventh and eighth requests using their private gateway correlation IDs.
The same provider name alone cannot establish the same upstream resource.
Client replay, plan persistence and HTTP 400 behavior have regression
coverage. The production routing outcome and a fresh live-model result
are still unverified.

## Integrated execution checklist (2026-11-19)

The integration candidate combines the task-progress base `bddb63f9`, the
large-page preview base `15e3214d`, and the staged local-tool scope changes
from the installed version. On macOS 26.5.1 arm64, typecheck, frontend and Rust
formatting, renderer production build, and all-targets Clippy passed. The full
frontend suite passed 3,369 tests (one skipped). Cargo passed 4,544 tests (28
ignored), including both-apex HTTP 400 persistence and the explicit Auto
PPTX case with no saved file. The bundled Node 22.23.4 / Pi 1.5.0 run passed
821 runtime and 102 search tests. A separate two-apex Host test verified a
read-only probe of the installed Codex CLI 0.166.0.

The 900 px browser fixture used the production checklist component with
synthetic events. It verified light and dark layouts and keyboard expansion.
An isolated ad hoc signed macOS 1.5.2 debug package started in a temporary
profile. Its main executable hash begins `24daadc8c75e`. Native window
automation could not attach to the window. The package's own Node/Pi and
OfficeCLI were exercised directly with a synthetic Host and intercepted
Chat Completions and Responses streams. Each same-session sequence answered
Fizzy, reported 0/3 through 3/3 and wrote a validated three-slide PPTX, then
answered a date question. Both simple turns produced zero plan events or
tool calls. This verifies the packaged runtime and file engine.

## Isolated real DeepSeek check (2026-11-19)

The signed macOS 1.5.2 debug package (`24daadc8c75e`) opened in the temporary
profile. The native UI submitted one request for a three-slide PPTX using
`deepseek/deepseek-v4.1-flash-1017` and the Cartesian style. Search and image
generation were disabled. Measured from the Send click, the first persisted
checklist appeared at 17.361 s and the failed reply settled at 21.671 s. The
native summary displayed 0/4 with the failure state. It produced no PPTX.

The first upstream request returned HTTP 200 and a successful `updatePlan`.
The second forward attempt had no upstream HTTP response in the record. The
test forwarder returned 502 and blocked later retries locally with 429. The
known usage at the undiscounted rate was US$0.0082902. With the unknown attempt
still reserved, the conservative total occupied US$0.7690902 of the authorized
US$3.00. No second user submission was made.

### Continuation investigation

The OFox request detail page matched the first request and its US$0.00829020
charge. No corresponding continuation was visible in the account's request,
failure, or blocked views. The installed Pi serializer reconstructed the
continuation offline, retaining the three successful tool-call/result pairs.
The original forwarder accepted that reconstructed body. Its incident record
shows the actual continuation was admitted locally and failed in roughly
260 ms before the forwarder captured an upstream HTTP status. The original
wire bytes and a production ingress trace are unavailable.

The private test forwarder now distinguishes connect/TLS, request-write,
response-header, and response-body failures, records safe upstream request IDs,
and stops on unknown outcomes without passing a successful end marker. Offline
regressions cover those stages and preserve the prior US$0.7690902 budget
occupation. Attribution of the historical disconnect still requires a gateway
ingress trace. No additional paid request was made during this investigation.

## Automatic checklist native acceptance (2026-11-19)

After the diagnostic forwarder was repaired, the same DeepSeek model completed
a three-slide PPTX in 147.723 s with six HTTP 200 responses, but omitted
`updatePlan` when the prompt simply requested the deck. This reproduced the
gap between optional planning guidance and the expected automatic checklist.

The updated isolated 1.5.2 package used the same Cartesian three-slide task
without a user instruction to create a checklist. The first persisted plan
was observed 5.766 s after clicking Send. Its four task-specific steps progressed
from 0/4 to 2/4 and then 4/4 at 186.435 s. The native UI showed two completed
rows and one active row during generation, and four checked rows at the end.
The reply completed at 197.221 s with one saved three-slide PPTX. The observer
polled persisted state every 15 ms; native UI checks confirmed the displayed
states independently.

This run recovered from one rejected raw-document call and retained one
reported difference between the planned visual structure and its final native
objects. The final PPTX was ZIP-valid with three actual slide parts and SHA-256
`a6f163207d216c12ea4f36a409b657d4284c8414d0cd3115afcf80d78a4de1b2`.
All ten upstream requests returned HTTP 200 with correlation IDs. Its token-price
upper estimate was US$0.0927546. The cumulative budget occupation, including the
original US$0.7608 unknown reservation and the prior runs, was US$0.9260622
against the authorized US$3.00 ceiling. Final invoice reconciliation and the
original pre-header disconnect's attribution remain open.

Independent review then reproduced a stale-picker edge case: an admitted local
diagnosis could finish its checklist while the PPT delivery gate changed its
last step back to pending. Both initial planning and delivery expectations now
exclude that admitted local scope. An actual successful presentation freeze can
still establish delivery expectations later. The final isolated package includes
this correction; packaged offline tests verified the diagnosis checklist and
attachment receipt gate. A packaged Chat Completions/Responses replay also
answered Fizzy, generated two actual three-slide OfficeCLI files with checklist
updates, and answered a simple date follow-up without a checklist. Restarting
the final native package restored the saved live-run file and all four checks.
The paid PPT timing above belongs to the preceding package; this last scope
correction was verified without another paid request.

Final checks passed: TypeScript, frontend formatting, 3,369 frontend tests
(one skipped), Rust formatting, all-target Clippy with warnings denied, and
4,544 Rust tests (28 ignored). The full bundled-runtime suite passed 829 tests
before the final stale-picker correction; 230 focused runtime tests passed
after it. Search tests passed 102 tests. These are macOS results; native Windows
verification and opening the final live PPTX in PowerPoint/WPS remain unverified.

## Generic task preparation native acceptance (2026-11-20)

The first isolated non-PPT attempt used DeepSeek V4.1 Flash 1017 and a synthetic
registration/payment reconciliation task. Its prompt requested CSV details and a
Markdown report without asking for a plan. Four model-authored steps appeared
38.013 s after Send. Two successful HTTP 200 model requests produced both actual
attachments. Before the next HTTP request could be written, the private test
forwarder's TLS connection failed with `SSLEOFError`. The reply ended in error at
66.352 s and kept its unfinished 0/4 checklist; it did not infer completion from
the saved files. This attempt is failure evidence, not a passing acceptance.
The US$0.7608 reservation was conservatively retained, bringing cumulative budget
occupation to US$1.7045937 against the original US$3 cap. A successor forwarder
must carry that entire amount and all earlier holds; no reservation is discarded
to make a new attempt fit the budget.

### General task promotion and final verification

A subsequent native UI run of the same non-PPT task automatically reported five
steps, progressing from 0/5 through 2/5 to 5/5. Its first plan appeared at
34.320 s and the reply completed at 96.950 s. The generated CSV had an unquoted
comma in its header, so that run did not pass strict file-content acceptance.
A same-chat arithmetic follow-up answered `50` in 5.286 s, using one model
request with no tool calls or plan events; the native UI showed no new checklist.

A repair attempt exposed a general execution boundary: after single-step
preparation and one saved file, the model attempted another operation without
creating a plan. The operation was refused, but optional guidance did not make
the provider reliably promote the task. The final runtime therefore names the
plan tool on the next normal request after an actual additional operation is
attempted. Offline regressions cover ordinary Chat Completions, Responses,
Anthropic, local tools, skill discovery and MCP, concurrent batches, malformed
planning, cancellation, steering and the shared tool-budget fence. Independent
review reproduced bounded termination and preserved budget exhaustion after
steering without additional paid traffic.

Final live acceptance used the actual production Rust `AssistantHost`, native
file writers and the signed isolated macOS 1.5.2 package's pinned Node 22.23.4 /
Pi SDK 1.5.0 runtime. The selected model remained
`deepseek/deepseek-v4.1-flash-1017`. The synthetic task read a registration/payment
CSV, recalculated each person's amounts, and saved a CSV plus a Markdown report;
its prompt did not ask for a plan. Measured from Host submission, the first
persisted plan appeared at 31.672 s, progressed from 0/5 through 3/5 to 5/5 at
246.940 s, and the reply completed at 252.503 s. These are Host event timings,
not GUI Send-click measurements. All ten model requests returned HTTP 200.

All three generated attachment receipts passed byte-length and SHA-256 checks;
the earlier CSV revision was retained. The latest CSV has fourteen data rows and
a header, all nine columns wide. Its six registration records, unmatched receipt
of 60, receivable total of 840, matched receipts of 730 and total receipts of 790
passed exact numeric checks. One further document revision was refused by the
existing three-document limit; the saved deliverables and completed plan were
preserved. This demonstrates planning, real execution, progress persistence and
file delivery. It does not establish complete business-report correctness.

Content review retained five issues: a CSV subtotal memo names A005 where A004
is intended; the Markdown report reverses the signed subtraction for a net
difference of -50; its final row-count description is stale; the A004 table mixes
matched and pending receipts; and a CSV summary says "verified" despite only
checking supplied synthetic records. Generated artifacts were not silently
corrected or described as fully accepted. General checklist completion remains
model-reported. The final assistant reply also gives a stale data-row count.

The final package's native window could not be attached after restart. Its
runtime and real Host execution were verified, but final GUI acceptance remains
open. Earlier GUI evidence covers the five completed checks and direct simple
answer before the last promotion correction. The main installed application,
its sessions and settings were not replaced by this isolated check.

The final runtime source SHA-256 values are
`3f83950b7247feea38cd1d6c0469eae24a2b9fbe073024c975e0aae31741b99b`
(`runtime.mjs`) and
`c675f39aba5220dd4833b867386c64d45740c96464230e0d916a187d8531aa77`
(`task-plan.mjs`); the prepared and packaged copies match. Forty dedicated
planning regressions also passed inside the isolated package.

The final run's undiscounted token-price upper estimate was US$0.0729663.
Cumulative budget occupation was US$1.8356472, including US$1.5216 in retained
historical unknown reservations, below the original US$3 cap. Paid testing was
stopped. This is a conservative budget ledger, not a reconciled invoice.

Final source verification on macOS 26.5.1 arm64, based on `bddb63f9`, passed
TypeScript, frontend formatting, 3,369 frontend tests (one skipped), 870 bundled
runtime tests and 102 search tests, plus Rust formatting, all-target Clippy with
warnings denied and 4,545 Rust tests (28 ignored). Independent implementation
and evidence reviews found no remaining blockers. The final local logs are
`ofox-frozen40-exact-{typecheck,format,unit,rustfmt,clippy,rust}.log` and
`ofox-frozen40-exact-{sidecar,search}-c2.log`. Native Windows execution remains
unverified.
