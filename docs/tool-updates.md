# Tool updates

Ofox follows [cc-switch's lifecycle implementation](https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/commands/misc.rs): resolve the executable used by the launch shell, update its owning installation, then check the effective version again.

## Supported behavior

- macOS: Claude Code, Codex CLI, Gemini, OpenCode, OpenClaw and Hermes version checks; single or sequential batch updates.
- npm installations: use the owning prefix's npm and Node.js, with an explicit `--prefix`. This also supports fnm/nvm installations whose shell entry points change between sessions.
- pnpm 11 installations: match the shim target against the manager's global bin and package root. Locally cached managers are considered when PATH points to an older manager. Reinstall only the selected package using `add --global <package>@latest --allow-build=<package>` and explicit global directories. This permits that tool's required postinstall, not arbitrary dependency scripts. Older layouts and mismatches fall back to manual guidance. Release-age and other safety policies remain enabled: registry latest may be newer than the version pnpm currently permits.
- Homebrew: update the owning formula/cask. Supported native installations use the CLI's own update command.
- Unknown installation sources (including Bun and Volta wrappers in this release): show a visible manual-update action instead of installing another copy.
- WorkBuddy and desktop-only Codex: use in-app updates or official download pages. App versions are not compared with CLI versions.
- Windows/Linux: version checks only; no automatic updates in this release.

The settings panel and console share update state. Tools with updates appear first, highlighted in orange, with an update count and an automatic or manual action. Checks are cached for five minutes between mounts and can be refreshed manually. A failed query never means “up to date.” Updates do not change Ofox bindings, API keys or model settings.

## API

`get_tool_versions` retains its existing arguments and adds `update_status`, `update_source`, `update_supported`, `update_reason` and `executable_path`. `includeLatest: false` still skips network queries.

`update_tool({ tool, operationId })` returns `{ status, before, after }`. Status is `updated`, `current`, `unchanged`, or `repaired`; execution errors reject the call. A found executable that fails its version probe has `update_status: broken`, not `notInstalled`. Verified pnpm 11 installations expose a repair action even without a runnable version. `repaired` confirms execution, not equality with the registry latest. The CLI's failed probe diagnostic is retained. `tool-update-progress` events contain `{ tool, operationId, stage, detail }`. Subscribe before invoking; ignore events for other operation IDs. Installation and update commands share a per-tool lock. Updates time out after 15 minutes and terminate their process group. Package-manager failures are not transactional; inspect diagnostics and repair before retrying normal use.

## Verification

Automated tests use mocked HTTP responses and shell processes; they never upgrade installed tools. Explicit manual acceptance tests are ignored by default:

```sh
cargo test --manifest-path src-tauri/Cargo.toml manual_probe_installed_tools --lib -- --ignored --nocapture
OFOX_TEST_UPDATE_TOOL=codex cargo test --manifest-path src-tauri/Cargo.toml manual_update_npm_tool --lib -- --ignored --nocapture
OFOX_TEST_UPDATE_TOOL=opencode cargo test --manifest-path src-tauri/Cargo.toml manual_repair_pnpm_opencode --lib -- --ignored --nocapture
```

The second command upgrades the actual selected npm installation and requires a version change. Run it only as an intentional acceptance check. Homebrew/native upgrades and Windows detection require platform-specific manual validation in addition to unit tests.

### Local acceptance (2026-09-22)

- macOS ARM64, fnm Node.js 24.15.0: Codex CLI upgraded from 0.154.0 to 0.155.1 using the actual Rust update implementation; the post-update launch-shell probe confirmed 0.155.1.
- Claude Code and Gemini were detected in their fnm/npm installation; Hermes was detected through its native launcher. No updates were performed for those tools.
- The initial OpenCode pnpm validation covered source detection only. The subsequent user upgrade exposed a skipped postinstall and a broken executable. Regression coverage now includes package-scoped build permission, failed-probe diagnostics (including misleading version strings in errors), and the repair UI. OpenClaw was not installed.
- The corrected Rust repair path was executed against that broken local installation with pnpm 11.19.0: OpenCode's postinstall completed, the result was `repaired`, and an independent login-shell probe returned **1.18.31**. Registry latest was **1.18.32**, published less than one day earlier; pnpm's default release-age policy was not bypassed. This verifies repair and execution, not installation of 1.18.32 or a clean-VM acceptance.
- Homebrew execution, native Hermes/Claude/OpenCode execution, and Windows detection have automated coverage where applicable but were not manually validated on clean VMs. This is not a release certification.
