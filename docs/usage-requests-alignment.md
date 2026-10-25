# Requests dashboard alignment

This change aligns only Usage → Requests with Magpie's request dashboard.
The final presentation has four summary cells, three chart metrics, two grouping dimensions,
stacked categories, ranking/day selection, and a horizontally scrolling request
table. Requests displays all four token buckets in its total. Overview,
session analysis, shared navigation/filters, source readers and pricing retain
their existing behavior.

## Initial source and build

- Branch: `codex/usage-requests-alignment`.
- Base: `release/1.3.7`, `57d90b1c2e7118caba29f561361801f4a40a33eb`.
- The initial implementation was uncommitted for review; no release version was changed.
- Captured source files: 1083.
- Source-manifest SHA-256: `c08172227511721a677c8682bc537259759f026e0b452722e45b8f25d851dbed`.
- At initial delivery, the renderer production build passed; no desktop package
  had yet been built or installed.
  The existing application was preserved at that stage. The original checkout
  and earlier usage worktree remain intact. The temporary preview server,
  source fixture and browser tab were
  removed after acceptance, and the viewport override was reset.

The delivery artifacts include `source-manifest.json`, verification logs and
before/after screenshots. The fingerprint covers the ordered relative paths and
SHA-256 hashes in that manifest; documentation and generated build assets are
outside it.

## Initial verification

Environment: macOS 26.5.1, arm64, 2026-10-25.

| Check                               | Result                                               |
| ----------------------------------- | ---------------------------------------------------- |
| TypeScript                          | Passed                                               |
| Prettier source check               | Passed                                               |
| Renderer build                      | Passed                                               |
| Full frontend suite                 | 174 files, 1531 passed, 0 failed; two workers        |
| Rust fmt                            | Passed                                               |
| All-targets Clippy, warnings denied | Passed                                               |
| Full Rust suite                     | 20 suites, 3434 passed, 0 failed, 14 defined ignores |
| Independent review and re-review    | Passed after regression fixes                        |
| Diff scope and whitespace           | Passed                                               |

The initial regression demonstrated the missing request dashboard, filters and
cache-inclusive aggregation. Further failing regressions reproduced small-cost
ranking scaling, unknown Other prices, asynchronous dimension/focus loss,
explicit-filter day invalidation and offscreen focus in long rankings.
The final guards also exercise obsolete-day recovery, both apexes, composite
provider identities, unknown provenance, frozen pages and matching complete CSV.
CSV retains its original 16 columns and appends the inclusive token total; the
four-source parity guard verifies both the old columns and the appended sum.

Browser checks use fictional IPC responses with live IPC blocked. They cover
1280px Chinese, 900px English and 620px Japanese, each in light and dark themes.
Document width stayed within the viewport. The table scrolls internally at
900px and 620px. Interactive checks confirmed provider grouping and focus survive
filtering, keyboard day selection restores chart focus, and a long-name item
scrolled to the ranking's end remains visible after filtering. Unknown prices,
empty data and missing timing retain distinct displays.

Screenshots include `before-1280-zh-light.jpg`, `after-1280-zh-light.jpg`,
the other width/language/theme combinations, long names, unknown prices and
empty data. They demonstrate renderer behavior, not real user activity.

Native Windows WebView rendering and new paid agent traffic were not exercised.
Generation speed continues to show no samples when the existing readers cannot
prove it; no native speed inference or publication is claimed. The initial code
delivery did not include signing or installation; the subsequently authorized
local replacement is recorded below.
See the [IPC contract](usage-analytics-contract.md#requests-detail-and-export)
and [accepted boundaries](known-limitations.md#request-dashboard-alignment).

## Subsequent local installation (2026-10-25)

The user subsequently authorized replacing the local application. The optimized
macOS arm64 build completed in 930.99 seconds with the existing Developer ID
identity. Deep/strict code-signature verification passed for the built, staged
and installed bundles. All 32 installed bundle files matched the signed build.
The captured source manifest remained unchanged throughout compilation and
installation. Version remains 1.3.7; no release was published and notarization
was not requested.

Installed executable SHA-256:
`0ca63fda2eecc4b8ef5f7fc4124663369fdff5212f8e481033b99197900dec22`.

A verified dated copy of the previous application and its original relocated
bundle were retained outside the checkout, with an installation manifest in
the local delivery artifacts. Replacement touched the application bundle only;
user configuration and history directories were not replaced.

The user completed the macOS security authorization in the system window.
Native acceptance then confirmed that the installed 1.3.7 application renders
existing local history in Requests: five summary cells, the cache-inclusive
token breakdown, metric and grouping controls, colored stacked categories,
rankings and the request table. Missing status and speed evidence remain
explicitly unavailable. The running process uses the installed executable;
its hash, all 32 bundle files and deep/strict signature were reverified.
The 1083 captured source files still match the build manifest.

`installed-before.jpg` records the previous application;
`installed-after.jpg` and `installed-chart.jpg` record the installed Requests
interface. The local delivery artifacts also contain
`native-runtime-verification.json` and the completed installation manifest.
These read-only native checks do not exercise new paid agent traffic or
Windows. Computer Use did not interact with the protected system authorization
window or receive the user's password.

## Page cleanup follow-up (2026-10-25)

The user reported an empty area below the Requests footer and requested removal
of output speed and provider presentation. The chart's absolutely positioned
screen-reader readout had no containing block inside the page's scroll owner.
At a 1280 × 720 synthetic viewport it extended the document to 771px despite the
720px application shell. Positioning the Requests chart section contains that
readout without changing shared scrolling or removing its live accessibility
feedback. The same document then stays at 720px; the footer retains about 16px
of ordinary bottom padding. The 620px, 900px and 1280px checks pass in light and
dark themes, with Japanese, English and Chinese respectively.

Requests now displays Token, requests, cost and cache hit rate in its summary.
The chart offers Token, cost and requests, grouped by model or Agent. Provider
filters, combined/provider grouping, the request-table column and the technical
detail field were removed, as were speed presentation and the older-backend
speed card. A stored speed preference falls back to Token. The existing IPC
provider and measurement fields remain compatible; no reader, pricing rule,
database schema, shared filter, Overview or session-analysis behavior changed.

Two failing unit regressions preceded the changes. The parent integration test
also verifies a real Agent tool-ID selection and focus restoration after an
asynchronous response. Independent review found the legacy measurement note's
remaining speed reference; all three translations were corrected and re-review
found no remaining actionable issue. Follow-up build, installation and native
acceptance evidence are recorded separately from the initial delivery artifacts.

The final follow-up gates pass: TypeScript, source formatting, renderer build,
all 174 frontend files / 1533 tests, Rust fmt, all-targets Clippy with warnings
denied, and all 20 Rust suites / 3434 tests (14 defined ignores). No native
Windows or new paid-agent-request validation is claimed.

Follow-up source fingerprint (1083 files):
`f738124bd20ec29d72378891d3a08613cf10a6c51da5dffdc577698932399b53`.
This manifest hashes each ordered path, NUL, file SHA-256 and newline. The
temporary preview source, server and browser tab were removed after checking;
delivery screenshots and measurement records remain outside the checkout.

The follow-up optimized macOS arm64 build completed in 1170.75 seconds and was
signed with the same Developer ID identity. All 32 staged and installed files
matched the signed bundle, and deep/strict signature verification passed. The
running installed executable matches SHA-256
`969f17fdee5e8a98df204a77add787e25a66d64193ce74df9c76a3b49a3cd55c`.
Version remains 1.3.7. The previous signed application was retained in a verified
dated backup and as an original relocated bundle; configuration and history
directories were not replaced. No notarization or publication was requested.

Installed-window acceptance confirmed four summary cells, three metrics,
model/Agent grouping, no provider column or selector, and the updated search
label against existing local history. The page footer now ends at ordinary
bottom padding while the application header stays visible; the extra outer
scrollbar and large empty area are gone. `installed-after-summary.jpg` and
`installed-after-bottom.jpg` capture the installed result. The export dialog
was opened and cancelled to reach the footer; no CSV file was saved. All 1083
source files still match the follow-up manifest after native acceptance.

## Branch installer distribution

The user subsequently authorized distributing new installers on the matching
branch while deferring an official Release. The verified Requests changes are
delivered through `release/1.3.7` and the existing Branch Packages workflow,
retaining version 1.3.7. Its macOS arm64 DMG and Windows x64 NSIS artifacts each
include the exact source commit, SHA-256 and build provenance. The workflow
requires complete CI and the independent Windows Probe for that same commit.
The workflow run and artifact records are the authority for remote completion.

These branch packages use ad-hoc signing without notarization on macOS and are
unsigned on Windows. They are distinct from the locally Developer ID signed
application checked above. Distribution creates no tag or official Release and
does not update production downloads. The local delivery records retain the
runtime source manifest and installed executable identity independently of the
branch artifacts.
