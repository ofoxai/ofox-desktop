# Assistant S4 progress — 2026-10-28

Tracking: Fizzy #1234.
Branch: `feat/assistant-chat`; starting revision `f493ce13`.

## Implemented in this increment

- Failed replies save the host's stable `errorCode` alongside the message. A
  restart or later merge of native history keeps the failure and its localized
  explanation, including the warning that an unconfirmed search may have been
  charged. Raw provider error text is not added to the session record. Successful
  and stopped replies do not retain an error code.
- Editing a question permits removing any or all of its attachments. Removing
  the last file returns keyboard focus to the editor. Unreadable files are left
  out, and the text can still be sent. An empty question without attachments
  cannot be submitted. The original message, its files and the unsent composer
  draft remain intact.
- `replaceAttachments: true` distinguishes an intentionally empty edit selection
  from the old client's empty list. Older clients still keep the original files;
  regeneration always uses the original turn's files. Older saved messages without
  `errorCode` remain readable and show the existing generic failed state.
- Updated the attachment-edit explanation in Chinese, English and Japanese.

## Verification

The new frontend failure-display tests and Rust reload assertion failed before
the fix. The frontend remove-last-file test and Rust explicit-empty selection
test also failed before implementation.

Regression coverage includes message events and a fresh host reloading saved
failures, native-history replay after a failure, successful/stopped outcomes,
legacy callers, empty edits, focus restoration, and the actual bundled runtime's
model input after removing an attachment. The last test runs both `ofox.ai` and
`ofox.io` identities against local mock gateways and verifies that the old
attachment's body is absent while the original message and draft survive. Every
write is confined to temporary configuration directories with synthetic keys.

Frontend: 1,921 tests passed; typecheck, formatting and renderer build passed.
Rust: 3,638 tests passed, 14 existing opt-in tests ignored; rustfmt and
all-targets Clippy with warnings denied passed. The assistant host's 25
integration tests ran the prepared Node 22.23.4 / Pi SDK runtime on macOS against
local mock gateways. These are the final checks after the code and regression
test changes; only documentation was updated afterwards.

Independent code review found no blocking findings. Its requested regression
coverage was added and the attachment-edit wording was corrected.

The screenshots below use the real renderer in a 900 × 600 fixture with a mock
backend. Click checks covered opening saved history, editing, removing the last
attachment, focus returning to the editor, and the explicit empty selection sent
to the API. These screenshots are not evidence of native Tauri login or real
gateway billing.

| Fixture                                    | Evidence                                                    |
| ------------------------------------------ | ----------------------------------------------------------- |
| Chinese, light: saved failure explanation  | [Screenshot](screenshots/assistant-s4/failure-zh-light.png) |
| English, dark: saved failure explanation   | [Screenshot](screenshots/assistant-s4/failure-en-dark.png)  |
| English, dark: removable edit attachment   | [Screenshot](screenshots/assistant-s4/edit-en-dark.png)     |
| Japanese, light: saved failure explanation | [Screenshot](screenshots/assistant-s4/failure-ja-light.png) |

## Office and compact composer increment

DOCX, XLSX and PPTX text extraction, native picker MIME/filter support, localized
outcomes and production packaging are implemented. The [Office audit](assistant-office.md)
records parsing limits, licenses, `.doc` rejection evidence and exact scope.

- Runtime tests: 143 passed plus 85 search tests; four subsequently added archive
  boundary/empty-document tests also passed (the Office suite now has 29 tests).
- Production resource smoke passed, including all three real generated Chinese
  Office files in the copied runtime with its pinned Node.
- Frontend full suite: 1,926 passed in 186 files. The previous generic unsupported
  message assertion was updated to the new Office and `.doc` conversion wording.
- Rust full suite: 3,640 passed, 14 opt-in tests ignored, 21 suites. All-targets
  Clippy, rustfmt, TypeScript, formatting and renderer build passed.
- Independent reviews found two content extraction issues (phonetic text and
  compatibility branches); both were reproduced, fixed and independently retested.
  The runtime, Rust/UI/packaging integration and final layout reviews found no
  remaining blockers.

The composer now occupies at most half the conversation panel's height. With
10 draft attachments in the 900 × 600 mock fixture, it shrank from 337 px to
271 px in a 541 px panel. Only the attachment list shrinks and scrolls; the editor,
Send/Stop, blocked-file hints and cost text remain visible. Shift+Tab from the
editor scrolls the final attachment button into view. The 200 page/attachment
component tests, typecheck, formatting and renderer build passed after this CSS
change. This layout check uses a browser fixture, not native Tauri.

| Fixture                         | Evidence                                                           |
| ------------------------------- | ------------------------------------------------------------------ |
| Office outcomes, Chinese light  | [Screenshot](screenshots/assistant-s4/office-zh-light.png)         |
| Office outcomes, English dark   | [Screenshot](screenshots/assistant-s4/office-en-dark.png)          |
| Office outcomes, Japanese light | [Screenshot](screenshots/assistant-s4/office-ja-light.png)         |
| Ten draft files, before         | [Screenshot](screenshots/assistant-s4/composer-before-en-dark.png) |
| Ten draft files, after          | [Screenshot](screenshots/assistant-s4/composer-after-en-dark.png)  |

## Attachment lifecycle and trash increment

- Import now commits attachment metadata and draft references together. Removing
  a draft file drops its metadata and bytes when no message (including superseded
  turns) references it, returning its count and size to the quota.
- Metadata is committed before unused bytes are removed. Startup cleanup reclaims
  crash leftovers and old unused metadata under the same attachment lock as
  imports. It does not follow links or touch unknown filenames. Active readers
  prevent cleanup. A failed cleanup returns the committed session with a separate
  localized warning so the model/search choice and host side effects stay aligned.
- Send admission and import own the draft while preparing. Older queued autosaves
  cannot delete their files; completion flushes the latest cached draft. Leaving
  and returning, cancellation, failed admission and already-finished replies are
  covered. Admission rechecks trash and file references before publishing a turn.
- The page lists active conversations or trash, preserves drafts when moving and
  restoring, and keeps trashed messages/drafts read-only. Permanent deletion has
  an explicit confirmation, with Cancel initially focused; Escape returns focus
  to Delete, and successful deletion returns focus to the Trash view.
- Deletion checks the confirmed owner and authority generation, plus trash/idle
  state under the file locks. It atomically moves the folder out of the session
  namespace, then removes native history, attachments and runtime files. Startup
  retries partial deletions; a failed item does not block other items. Unrelated
  directories and link targets are preserved.

Regressions were reproduced before fixes for draft removal, import publication,
admission revalidation, post-commit cleanup errors, delayed send/import autosaves,
selection away/back, cancelled draft saving, finished-run recovery, stale list
refreshes, interrupted deletions and dialog focus. Independent reviews of both
attachment cleanup and trash found no remaining blocking findings.

Final checks for this increment passed: 1,940 frontend tests in 186 files;
3,657 Rust tests across 21 suites, with 14 existing opt-in tests ignored;
TypeScript, formatting, renderer build, rustfmt and all-targets Clippy with
warnings denied. The first full run caught the error-code guard's flat directory
scan (now recursive for the new store submodule) and three redundant test borrows;
both were fixed and the full affected gates rerun successfully.

Browser checks used the real renderer in a 900 × 600 **mock** fixture: Chinese
light, English dark and Japanese light, ten draft attachments, move/restore,
read-only state, default Cancel focus, Tab, Escape, focus return and a simulated
delete. Restoring retained all ten files and the draft text. No real user data
was deleted. These checks do not establish native Tauri or Windows acceptance.

| Fixture | Evidence |
| --- | --- |
| English dark, trash | [Screenshot](screenshots/assistant-s4/trash-en-dark.png) |
| English dark, confirmation | [Screenshot](screenshots/assistant-s4/trash-confirm-en-dark.png) |
| Chinese light, trash | [Screenshot](screenshots/assistant-s4/trash-zh-light.png) |
| Chinese light, simulated deletion | [Screenshot](screenshots/assistant-s4/trash-empty-zh-light.png) |
| Japanese light, trash | [Screenshot](screenshots/assistant-s4/trash-ja-light.png) |

## Remaining scope

S4 implementation and macOS native acceptance are complete, including the
user-confirmed permanent deletion of a dedicated synthetic session and disk
verification. Windows was explicitly deferred by the user on 2026-10-28.
See [native acceptance](assistant-native-acceptance.md).
No installer or release was published.

## Markdown export and aggregate usage increment

Markdown export uses a fresh saved snapshot after flushing the unsent draft. It
preserves message Markdown and code, safe HTTP(S) source links, attachment names
and reading limits, localized failures and precise decimal costs. It exports the
current branch; a complete backup preserves earlier versions and file bytes.
The native save dialog can be cancelled without an error. Confirmed account and
authority generation are rechecked after the dialog; managed keys are removed,
application data and symlink destinations are refused, and the file is atomically
saved with private permissions. The export is bounded to 64 MiB.

The sidebar now reads `usage_summary` for all conversations including the trash;
searching or changing the visible list does not filter the total. Unknown,
interrupted or unreadable usage has no claimed price. Starting a reply clears the
old total price and finishing refreshes it. Old replies and previous accounts
cannot replace the current totals.

Focused regressions passed for draft flushing, save cancellation/failure,
selection/account changes, native private writes, key scrubbing, destination
protection, exact prices, asynchronous totals and owner/trash isolation. Review
found and fixed a Markdown ambiguity: an unconfirmed search could show a known
count of zero alongside an unknown cost. Counts now appear only with a complete
known estimate. Independent reviews of export and aggregate usage passed.
TypeScript, formatting and renderer build passed before backup work began. The
full final gates will be rerun after backup/restore is integrated.

The 900 × 600 browser fixture checked English dark and Chinese/Japanese light,
ten draft files, keyboard export, preserved draft content, cancellation, unknown
cost and focus staying on Export. These use mock native save replies; actual
native dialogs and Windows remain pending.

| Fixture | Evidence |
| --- | --- |
| English dark, saved export | [Screenshot](screenshots/assistant-s4/export-usage-en-dark.png) |
| Chinese light, cancelled export and unknown cost | [Screenshot](screenshots/assistant-s4/export-cancel-usage-zh-light.png) |
| Japanese light, saved export | [Screenshot](screenshots/assistant-s4/export-usage-ja-light.png) |


## Complete backup/restore increment and final local gates

The [backup contract](assistant-backup.md) documents the owner-scoped ZIP format,
limits, atomic writes and conflict policy. Backups include drafts, trash, old
message branches, native history, attachment bytes and the original operation
prices. Restore adds missing sessions, skips identical content and never overwrites
conflicting conversations. Validation precedes publication; partial publication
returns counts and a warning, preserving valid imports for an idempotent retry.
Native save/open dialogs enforce confirmed owner and authority generation and
cancel quietly. Staging cleanup is confined to recognized private folders.

Independent review found and fixed two native-format issues: valid large image
JSONL lines were incorrectly bounded at 8 MiB, and malformed Pi entries were
accepted without checking message shape/run lineage. Review also found draft loss
when switching during creation of a backup session and restored model preferences
not being loaded into the renderer. Regression tests reproduced and fixed these;
a failed preference reload also blocks stale-default creation until refresh.
The additional unchanged-draft test passed without a code change: the existing
saved baseline already avoids altering identical conversations before restore.
Final independent reviews passed, including account/reset/late completion paths.

The actual prepared Node 22.23.4 / Pi SDK 1.4.0 runtime continued after backup,
removal of the original temporary config, restore into an empty config and host
restart. Both apex identities used local mock gateways. Restored history and the
attachment were read successfully, and original operation/search prices remained
unchanged despite a changed catalog price. This is runtime integration evidence,
not native desktop clicks or real-gateway billing verification.

Final gates on macOS 26.5.1 arm64, based on `f493ce13` plus this uncommitted S4
increment, all passed after the final code change:

- 1,965 frontend tests in 187 files; TypeScript, Prettier and renderer build.
- 3,674 Rust tests across 21 suites; 14 existing opt-in tests ignored; rustfmt and
  all-targets Clippy with warnings denied.
- 147 runtime tests and 85 search tests on pinned Node; production resource smoke
  including Pi, native canvas, PDF, native JSONL, Office extraction and search.
- `git diff --check`; independent final review with no blocking findings.

The browser fixture covered backup with the latest draft and ten files, exact
restore/skipped counts, partial success with a conflict warning, cancellation,
keyboard Enter/Tab, focus staying on the initiating control and no horizontal
overflow at 900 × 600. Native dialog replies were mocked. The Chinese cancellation
case kept its draft and files with no new transfer error or success message.

| Fixture | Evidence |
| --- | --- |
| Japanese light, saved backup | [Screenshot](screenshots/assistant-s4/backup-ja-light.png) |
| Japanese light, restored/skipped counts | [Screenshot](screenshots/assistant-s4/restore-ja-light.png) |
| English dark, partial restore warning | [Screenshot](screenshots/assistant-s4/restore-partial-en-dark.png) |
| Chinese light, cancelled restore | [Screenshot](screenshots/assistant-s4/backup-restore-cancel-zh-light.png) |

The subsequent native pass below supersedes the pending native status above.
Windows is deferred by the user; the native acceptance record includes the
completed disposable-session deletion check.


## macOS native acceptance increment

The actual Tauri application now has the native click evidence described in
[the acceptance record](assistant-native-acceptance.md). It runs an isolated
bundle and temporary profile with local synthetic authorization/key/model/search
services. The public catalog still follows the selected apex. Earlier live
Key/gateway checks retain their original scope; no additional production charge
or per-request account-balance reconciliation is required.

Native checks passed for chat/context, Markdown/code, text/PDF/image/Office file
selection and real runtime inputs, MCP sources and search count, stop/regenerate,
model selection, restart/failure/retry, edit with all files removed, history search
and rename, trash/restore, native Markdown/ZIP saves and cancellation, identical
restore skips, empty-profile restore, and continuation with an actual
`read_attachment` of the restored old file. Six attachment files and native JSONL
are byte-identical, original usage/operations are unchanged, and a later turn does
not rewrite historical operation prices. Ten draft files, keyboard Shift+Tab to
the last file, and the 900×600 native dark window also passed.

Changing `.ai` to `.io` cleared the old account view; logged-out local tools stayed
available. The normal native device authorization flow completed against the
local mock service and a new `.io` conversation ran successfully. The main test
conversation and its ten-file draft stayed intact.

Independent acceptance review identified the final native check: permanently
deleting a dedicated synthetic two-message/one-file session. After the user
confirmed, the actual UI deletion succeeded. The session/native/attachment
directories and deletion tombstone are absent. The main conversation, ten-file
draft, original profile copy and byte-identical ZIP backup remain available.
Original operation prices were also preserved. All macOS acceptance checks in
this scope are complete; Windows is deferred at the user’s explicit request,
and the last acceptance step has been updated to say so.
No product code changed during this acceptance pass, so the final local test gates
above remain applicable.
