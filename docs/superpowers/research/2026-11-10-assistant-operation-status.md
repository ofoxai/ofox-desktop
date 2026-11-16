# Tool responses and task completion — 2026-11-10

The first conversation-progress UI used the same check icon for a successful tool
response and a completed model task step. In the reported case it showed style,
guidance and style again as three checked items while the model was still
thinking. That rendering overstated what the available evidence established.

## Verified cause

`get_presentation_style` without an ID lists styles; with an ID it returns a
recipe/theme/provenance. Neither operation commits a user choice. The renderer's
sanitized event does not preserve that ID or a selected-style result. A successful
`tool_end` establishes a returned call, not a selected style, an accepted business
milestone, verified content quality or a delivered PPT. The shared category label
previously said “Choose a presentation style” and the icon showed a check.

## Correction

- Operation records use neutral dots and visible Returned labels; the style
  category now says Query presentation styles in Chinese, English and Japanese.
  Checklist checks remain reserved for model-reported completed task steps.
- Deduplicate calls by ID with the latest status winning, then group by category
  and status in latest-occurrence order. Repeated queries show recent call counts.
  Counts describe only the hook's retained four-call window, not total run usage.
- Returned, running, failed and approval-waiting states stay distinct. A plan can
  hide returned records, while actual running calls and failures remain visible.
  Deduplication happens before this filtering so a newer return cannot resurrect
  an earlier running/failed state for the same call.
- The operation explanation distinguishes responses from task steps. No delay,
  synthetic work phase, percentage or new task-completion event is manufactured.

Only the progress component and three locale files change production behavior.
Generation, model prompts, runtime events, permission policy and Rust/backend code
are unchanged. The previous [progress report](2026-11-10-assistant-conversation-progress.md)
records the earlier installed version; its operation check examples are superseded
by this correction.

## Validation

A regression using the reported three-call sequence first failed with three rows
instead of two. Ten new cases cover grouping, deduplication, latest ordering,
status separation, paused approval, plan coexistence and all three languages.
The focused new and existing progress suites pass 21 cases. Independent read-only
review found no blocking issue. Actual components were visually inspected at
900px in light and dark synthetic fixtures, with no account, IPC or model access.
The neutral operation records and the separate model checklist were both observed.

Full frontend validation passed 3,161 cases with one existing skipped test
(253 passed files and one skipped file). TypeScript, source/test/document formatting
and diff checks passed. Packaging and local-installation results are recorded
after completion. No new model generation is submitted for this correction. Native
Windows presentation remains unverified.

## Installed application

The signed macOS bundle passed strict signature/Team ID/minimal entitlement
verification, 224 source/resource hash checks and the SDK/PDF/OfficeCLI/search/
Typst offline smoke with five fixed-code style samples. The separate native-math
packaging probe passed. These checks establish packaging integrity, not model
output quality or new equation-editing acceptance.

Before replacement the existing app was checked idle, then exited normally. A
private APFS-clone backup retained its data and the previous application. The app
was replaced atomically and opened successfully; the user's latest Transformer
conversation, model catalog and composer loaded. No generation was submitted,
no dependencies were installed, and no commit or push was made. Version remains
`1.5.0`. Signed local development packaging is not a notarized public release.

Installed executable SHA-256: `62e2058608a19e2d9790975e5610c9991a53c410c739d176cf0922d7ee58487e`.

## Subsequent user preference: check icon for a returned operation

The user requested replacing visible Returned text with a check icon. This now
appears on the right of returned operation records, with the localized Returned
accessible name. Accurate query labels, recent counts, grouping, and separate
model-task completion semantics are retained. Running/failed badges are unchanged.
The prior neutral-dot-only/visible-Returned requirement is superseded by this
explicit preference; it is not a new business-completion event.

The requested icon revision passed 21 focused cases and the full frontend suite
(3,161 passed, one existing skip), TypeScript, formatting and diff checks, and
independent review. The signed package passed the offline runtime/resource and
math probes, including 224 source/resource hashes. The local application was
backed up, replaced and launched; assistant history, catalog and composer were
available. No model generation was submitted, dependencies installed, or code
committed/pushed. Installed executable SHA-256: `a0c741a4efd4fbaa2ece67c2d307218ffd8d610aa18e77dc69689a406599bfb7`.
