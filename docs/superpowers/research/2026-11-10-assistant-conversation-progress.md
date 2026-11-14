# Assistant conversation progress — 2026-11-10

## Scope and implemented behavior

This renderer change addresses long-running conversation presentation. It leaves
PPT design resources, Python/OfficeCLI generation, native formula construction,
permission policy and database schema unchanged. The supplied ofox-agent images
were visual/workflow references only; no closed-source code or assets were used.

- A live task surface appears before the model publishes a plan. Its current
  activity follows observed tool events; pending approval takes precedence over
  execution, reasoning and retry. Total elapsed time is separate from thinking
  duration and is not announced every second to screen readers.
- Before a model plan exists, up to four recent supported tool calls show their
  actual running/completed/failed outcomes. With a plan, recent observed failures
  remain visible because a model checklist has no failed state. This is a bounded
  activity summary, not a complete audit trail or a measure of content quality.
- Model-authored steps show completed/total, checks, an outlined current row and
  pending circles. The checklist is keyboard collapsible; waiting for approval
  pauses execution animation. Completed reported steps do not establish delivery
  or successful visual review.
- Raw execution intent and optional reasoning are collapsed details. A 45-second
  interval without a semantic stage update displays a quiet-wait explanation;
  tokens alone do not establish a new task stage. No estimate or percentage is
  invented. Unknown tools retain generic working status.
- Progress is scoped by account, authority generation, session, run and operation.
  It clears on teardown and is not written into historical messages. Older chats
  do not gain fabricated task plans.
- Streamed and saved replies now use one keyed transcript. A document preview
  retains its open state and current page across final-message/history handoff,
  including versioned replies and delayed history refresh. Existing session and
  authority boundaries still remount media. Focus returns to the composer only
  when a disappearing progress surface held focus.
- Newly observed activities also trigger the existing follow-to-bottom behavior;
  readers who scrolled up keep their position and receive the new-content hint.
- New visible labels cover Chinese, English and Japanese, using existing theme
  tokens, reduced-motion behavior and accessible list/count descriptions.

## Verification and evidence

Meaningful failing regressions were captured before implementation for early
progress, checklist controls, hook state and preview continuity. Final checks and
local installation results are recorded below after completion.

Visual inspection used actual renderer components with self-authored synthetic
conversation/plan inputs. The isolated fixture blocked IPC, outbound requests,
real storage and model access. Browser checks covered 900px light/dark layouts,
long unbroken labels, a 1400px layout, before-plan activity, approval, keyboard
collapse/expand, all-reported-steps-completed while still running, and timeout.
Screenshots and logs remain in the ignored local evaluation directory. Fixture
elapsed times are synthetic inputs, not measured model performance.

## Final source gates

- `pnpm typecheck`, `pnpm format:check`, targeted test/document formatting and
  `git diff --check`: passed.
- Full frontend: 3,151 passed, one existing skipped test (252 passed files,
  one skipped file). Final placeholder/version and activity-scroll fixes are included.
- Rust formatting and Clippy (`--all-targets -- -D warnings`): passed.
- Full Rust: 4,429 passed, 24 ignored, no failures across 22 suites. Ignored
  probes/platform checks are not counted as passes.
- Independent reviews covered hook authority/event scope, task presentation,
  accessibility and final stream/history/version identity handling.

## Local package and installed-app verification

The final macOS app was built with the existing Developer ID identity and
replaced the local installation after a clean exit and a private APFS-clone data
backup. Application version remains `1.5.0`; no release/version bump, dependency
installation, commit or push was performed. The backup retains the prior app;
user data was preserved in place, not restored or replaced.

- Signed-package verification passed: main and engine/native-addon signatures,
  Team ID and existing minimal entitlements, 224 source/resource hashes, SDK/PDF/
  OfficeCLI/search/Typst smoke and five fixed-code native style samples.
- The separate offline native-math compatibility probe passed. Neither probe
  used a live model or real credentials; these are packaging checks, not new
  model-output quality or equation-editing acceptance.
- The installed app opened successfully. Account/tool display, model catalog,
  existing conversation and its 10-page PPT card loaded. Native pagination showed
  pages 1 and 5, including the existing formula fallback; Escape closed the dialog
  and returned focus to its preview trigger. No generation was submitted.
- New installed executable SHA-256: `2ddbc1c772accbccccd3ee18285f13760d64277f720528a2ec81dd752697b262`.

## Remaining limits

No new paid model generation was used to validate this UI change. The prior real
Transformer run remains separate [evidence](2026-11-10-ppt-transformer-live-validation.md).
Model plan timing, repeated approval frequency, unrelated local-verification
continuations and historical reply-details duration/cost reporting are not fixed
by this presentation change. This card cannot expose work the host never reports.
Active plans/activity summaries are ephemeral across app restart. Native Windows
rendering and live-model event ordering on Windows remain unverified.

## Follow-up: operation response semantics

The initial operation check icons and “Choose a presentation style” wording were
too strong: a returned style lookup does not establish a committed choice. The
[operation-status correction](2026-11-10-assistant-operation-status.md) supersedes
that presentation with neutral Returned records and scoped recent counts. The
original screenshots and gate counts above remain historical evidence.
