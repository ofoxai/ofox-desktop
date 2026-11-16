# Compact file preview, 2026-11-11

This increment simplifies the right file pane in the existing development
checkout. It does not certify new PPT generation quality or replace the installed
application. Prior uncommitted PPT/runtime work is retained.

## Behavior

- One file heading with More, expand/restore and hide. A file-switch chevron
  appears only for multiple files. The separate file-picker and version rows
  are removed from the default layout.
- PPTX defaults to the fitted page and previous / current-total / next controls.
  Clicking the counter opens page jump. Thumbnails, zoom, references, visual
  draft preparation, reload and version details remain available on demand.
- Chat stays mounted through expand, hide, menu use and file switching. Quote
  and visual actions only prepare a draft. They do not send or generate.
- Existing page/version/authority guards, page limits, cached rendering and HTML
  isolation remain. Missing HTML resources, script errors, preview failures and
  genuine page limits remain visible. Technical notices are available in details.
- English, Chinese and Japanese labels are covered. Toolbar menus allow a
  deliberate focus transfer to the composer; heading focus survives replacement
  of the loading view by the ready preview.

## Evidence and limits

[Evidence directory](../../../.real-eval/ppt-preview-ui-simplify-2026-11-11/)
contains before-edit copies, initial failures, corrected regressions, final logs,
source hashes and a UI-only patch. The preview suite has 29 tests, including true
70-page navigation, late-response/authority guards, parent-version checks, HTML
isolation/error handling, keyboard focus, narrow layout and locale coverage. The
chat integration tests follow the new menu entry and still verify retained
streaming history, drafts and explicit file references without automatic sends.

macOS arm64 verification at base HEAD `2ebe649158fe6cbacb81cf740ea5ca8ada3ddc17`
with the preserved dirty working tree:

| Check | Result | Evidence log |
| --- | --- | --- |
| TypeScript / Prettier | Passed | `typecheck-verified.log`, `format-verified.log` |
| Production renderer build | Passed; existing chunk/import warnings retained | `build-renderer-complete.log` |
| Complete frontend suite | 3332 passed, 0 failed, 1 skipped; 264 files passed, 1 skipped | `unit-verified.log` |
| Rust formatting / all-targets Clippy | Passed | `rust-fmt-verified.log`, `clippy-verified.log` |
| Complete Rust suite, unchanged Rust source | 4514 passed, 0 failed, 27 ignored across 22 summaries | `rust-test-final.log` |

The earlier four chat integration failures used the removed direct quote button;
they passed after following More → Quote instead. The controlled file-switch
focus regression failed before heading focus was carried across loading/ready
replacement, then passed. Earlier failing logs remain in the evidence directory.

The CUA browser ran the actual renderer component with mocked IPC and eight
already rendered JPEG pages from a prior case, not a native preview backend or a
new generation. Observed: fit, page jump, keyboard paging, expand/restore, hide,
chat draft retention, quoting, file switching, menu Escape, details Close,
900px light/dark and 650px file/chat switching. See the
[default screenshot](../../../.real-eval/ppt-preview-ui-simplify-2026-11-11/default-light.png)
and the final keyboard-focus screenshot in the same directory. These screenshots
do not establish upstream-template completeness or PPTX editability.

No paid calls, commits, pushes or installation occurred in this increment.
The earlier V8b candidate ZIP does not contain this UI change. Installed Tauri,
PowerPoint/Windows and a new independent review remain unverified for this
increment. Existing general PPT visual issues remain in
[Fizzy #1615](https://app.fizzy.do/6179973/cards/1615); the overall PPT topic stays open.
