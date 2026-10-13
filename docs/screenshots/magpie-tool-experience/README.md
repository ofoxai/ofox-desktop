# Renderer verification screenshots

These screenshots use synthetic account, model, session, and IPC data in a browser. They demonstrate layout and renderer interaction only. No native application, real CLI request, personal tool configuration, or live account was used. The temporary fixture pages and baseline modules were removed after capture.

- `console-before.jpg`: ConsolePage and BoundToolRow from `origin/main` commit `71c885f8d64df6b24413a63794e671e650292e2e`, rendered with the same synthetic model values and the eight original tools.
- `console-after.jpg`: tool rows with quick model controls, separate observed/default ChatGPT models, OpenCode observed model, and Pi/dsh entries.
- `console-model-picker.jpg`: searchable single-model picker at 880 px; the large catalog scrolls and pages.
- `profiles-preview.jpg`: shared Codex/ChatGPT target, ordered multi-model selection, and affected dsh profiles.
- `profiles-partial.jpg`: per-tool results after a simulated write failure stops subsequent targets, with undo available for the completed write.
- `sessions.jpg` and `sessions-trash.jpg`: synthetic project groups, model/subagent metadata, messages, and reversible file trash. Dates are synthetic fixture values.
- `tray-multi-picker.jpg`: English dark-mode model set editor and scrollable catalog at a 360 × 620 viewport; portal content remains visible outside the dialog body.

The renderer also verified Japanese ChatGPT-only capture: the saved recipe contains one canonical `codex` target, while the observed App model remains separate from the writable shared default.
