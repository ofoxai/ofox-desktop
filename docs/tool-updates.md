# Tool updates

Ofox follows [cc-switch's lifecycle implementation](https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/commands/misc.rs): resolve the executable used by the launch shell, update its owning installation, then check the effective version again.

## Supported behavior

- macOS: Claude Code, Codex CLI, Gemini, OpenCode, OpenClaw and Hermes version checks; single or sequential batch updates.
- npm installations: use the owning prefix's npm and Node.js, with an explicit `--prefix`. This also supports fnm/nvm installations whose shell entry points change between sessions.
- pnpm installations: match the shim target against the manager's global bin and package root. Locally cached pnpm 11 managers are considered when PATH points to an older manager. Update only the selected package with explicit global directories; mismatches fall back to manual guidance.
- Homebrew: update the owning formula/cask. Supported native installations use the CLI's own update command.
- Unknown installation sources (including Bun and Volta wrappers in this release): show a visible manual-update action instead of installing another copy.
- WorkBuddy and desktop-only Codex: use in-app updates or official download pages. App versions are not compared with CLI versions.
- Windows/Linux: version checks only; no automatic updates in this release.

The settings panel and console share update state. Tools with updates appear first, highlighted in orange, with an update count and an automatic or manual action. Checks are cached for five minutes between mounts and can be refreshed manually. A failed query never means “up to date.” Updates do not change Ofox bindings, API keys or model settings.

## API

`get_tool_versions` retains its existing arguments and adds `update_status`, `update_source`, `update_supported`, `update_reason` and `executable_path`. `includeLatest: false` still skips network queries.

`update_tool({ tool, operationId })` returns `{ status, before, after }`. Status is `updated`, `current`, or `unchanged`; execution errors reject the call. `tool-update-progress` events contain `{ tool, operationId, stage, detail }`. Subscribe before invoking; ignore events for other operation IDs. Installation and update commands share a per-tool lock. Updates time out after 15 minutes and terminate their process group.

## Verification

Automated tests use mocked HTTP responses and shell processes; they never upgrade installed tools. Explicit manual acceptance tests are ignored by default:

```sh
cargo test --manifest-path src-tauri/Cargo.toml manual_probe_installed_tools --lib -- --ignored --nocapture
OFOX_TEST_UPDATE_TOOL=codex cargo test --manifest-path src-tauri/Cargo.toml manual_update_npm_tool --lib -- --ignored --nocapture
```

The second command upgrades the actual selected npm installation and requires a version change. Run it only as an intentional acceptance check. Homebrew/native upgrades and Windows detection require platform-specific manual validation in addition to unit tests.

### Local acceptance (2026-09-22)

- macOS ARM64, fnm Node.js 24.15.0: Codex CLI upgraded from 0.154.0 to 0.155.1 using the actual Rust update implementation; the post-update launch-shell probe confirmed 0.155.1.
- Claude Code and Gemini were detected in their fnm/npm installation; Hermes was detected through its native launcher. No updates were performed for those tools.
- The local OpenCode installation uses pnpm 11 while PATH selects pnpm 10. Source detection now selects the cached pnpm 11 manager owning the existing installation. The actual OpenCode upgrade is left for the UI action. OpenClaw was not installed.
- Homebrew execution, native Hermes/Claude/OpenCode execution, and Windows detection have automated coverage where applicable but were not manually validated on clean VMs. This is not a release certification.
