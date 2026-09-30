# Tool updates

Ofox follows [cc-switch's lifecycle implementation](https://github.com/farion1231/cc-switch/blob/main/src-tauri/src/commands/misc.rs): resolve the executable used by the launch shell, update its owning installation, then check the effective version again.

## Supported behavior

- macOS: Claude Code, Codex CLI, Gemini, OpenCode, OpenClaw and Hermes version checks; single or sequential batch updates.
- npm installations: use the owning prefix's npm and Node.js, with an explicit `--prefix`. This also supports fnm/nvm installations whose shell entry points change between sessions.
- pnpm 11 installations: match the shim target against the manager's global bin and package root. Locally cached managers are considered when PATH points to an older manager. Reinstall only the selected package using `add --global <package>@latest --allow-build=<package>` and explicit global directories. This permits that tool's required postinstall, not arbitrary dependency scripts. Older layouts and mismatches fall back to manual guidance. Release-age and other safety policies remain enabled: registry latest may be newer than the version pnpm currently permits.
- Homebrew: update the owning formula/cask. Supported native installations use the CLI's own update command.
- Unknown installation sources (including Bun and Volta wrappers in this release): show a visible manual-update action instead of installing another copy.
- ChatGPT desktop app (bundle `com.openai.codex`, MSIX `OpenAI.Codex`): the latest version comes from OpenAI's own update feeds (see [Fixed URL exceptions](#fixed-url-exceptions)). Versions are compared as dotted numbers within one platform, never as semver and never across platforms (MSIX versions have four parts and a different third part).
  - macOS: status only. When an update is available the card's "Update in ChatGPT" button opens (or activates) ChatGPT, whose built-in Sparkle updater installs it from ChatGPT → "Check for Updates…". Ofox never quits ChatGPT or sends it Apple Events, which would trigger a macOS Automation permission prompt.
  - Windows: one-click upgrade from the ChatGPT card. If ChatGPT is running, Ofox asks first, closes it (`CloseMainWindow`, then `Stop-Process` after 10 s), runs `winget upgrade --id 9PLM9XGG6VKS --source msstore`, and falls back to the Microsoft Store page when winget has no applicable update (`0x8A15002B`), is missing, or fails. It then waits for the installed version to increase (90 s after winget, 5 minutes via the Store) and reopens ChatGPT if it had been running.
  - ChatGPT is never part of "Update all"; it upgrades only from its own card.
  - OpenAI rolls releases out gradually. The public feeds can be ahead of what a given installation is offered, so ChatGPT may still report itself up to date; the card and `update_reason` say so.
- WorkBuddy and desktop-only Codex: use in-app updates or official download pages. App versions are not compared with CLI versions.
- Windows/Linux: version checks only; no automatic updates in this release, except the ChatGPT desktop app on Windows.

The settings panel and console share update state. Tools with updates appear first, highlighted in orange, with an update count and an automatic or manual action. Checks are cached for five minutes between mounts and can be refreshed manually. A failed query never means “up to date.” Updates do not change Ofox bindings, API keys or model settings.

## API

`get_tool_versions` retains its existing arguments and adds `update_status`, `update_source`, `update_supported`, `update_reason` and `executable_path`. `includeLatest: false` still skips network queries; for ChatGPT it reports `unchecked`. `update_source` is `homebrew`, `npm`, `native` or `pnpm` for CLIs and `sparkle` (macOS) or `msstore` (Windows) for ChatGPT. If ChatGPT's latest-version lookup fails, the status stays `appManaged` and `update_reason` carries the error. ChatGPT on Linux reports `unsupported`.

`is_tool_app_running({ tool })` (ChatGPT only) tells the renderer whether an upgrade would have to close the app; it returns `false` outside Windows. The renderer asks for confirmation when it is running or when the check fails.

`update_tool({ tool, operationId })` returns `{ status, before, after }`. Status is `updated`, `current`, `unchanged`, or `repaired`; execution errors reject the call. A found executable that fails its version probe has `update_status: broken`, not `notInstalled`. Verified pnpm 11 installations expose a repair action even without a runnable version. `repaired` confirms execution, not equality with the registry latest. The CLI's failed probe diagnostic is retained. `tool-update-progress` events contain `{ tool, operationId, stage, detail }`. Subscribe before invoking; ignore events for other operation IDs. Installation and update commands share a per-tool lock. Updates time out after 15 minutes and terminate their process group. Package-manager failures are not transactional; inspect diagnostics and repair before retrying normal use.

## Verification

Automated tests use mocked HTTP responses and shell processes; they never upgrade installed tools. Explicit manual acceptance tests are ignored by default:

```sh
cargo test --manifest-path src-tauri/Cargo.toml manual_probe_installed_tools --lib -- --ignored --nocapture
OFOX_TEST_UPDATE_TOOL=codex cargo test --manifest-path src-tauri/Cargo.toml manual_update_npm_tool --lib -- --ignored --nocapture
OFOX_TEST_UPDATE_TOOL=opencode cargo test --manifest-path src-tauri/Cargo.toml manual_repair_pnpm_opencode --lib -- --ignored --nocapture
```

The second command upgrades the actual selected npm installation and requires a version change.

ChatGPT desktop:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --lib manual_live_chatgpt_feeds_parse -- --ignored --nocapture
cargo run --manifest-path src-tauri/Cargo.toml --bin verify_chatgpt_install -- --check-latest
```

The first parses the live feeds to catch format changes. `verify_chatgpt_install` also accepts `--running` and, on Windows, `--upgrade` (closes, upgrades and reopens ChatGPT); the Windows Probe workflow publishes it as an artifact.

## Fixed URL exceptions

These are OpenAI infrastructure URLs, not Ofox endpoints, so they are exempt from the region-aware apex rule in `AGENTS.md`:

- `https://persistent.oaistatic.com/codex-app-prod/appcast.xml` (Apple Silicon) and `…/appcast-x64.xml` (Intel): the Sparkle feed ChatGPT itself uses (`codexSparkleFeedUrl`).
- `https://persistent.oaistatic.com/codex-app-prod/windows-store-update.json`: the manifest ChatGPT's production build uses for its Store update check. Ofox accepts it only for Store product `9PLM9XGG6VKS` / package `OpenAI.Codex`.

The personalized endpoint `updates.oaistatic.com/codex/app/appcast?installation_id=…` is deliberately not used: it needs ChatGPT's private installation ID. Run it only as an intentional acceptance check. Homebrew/native upgrades and Windows detection require platform-specific manual validation in addition to unit tests.

### Local acceptance (2026-09-22)

- macOS ARM64, fnm Node.js 24.15.0: Codex CLI upgraded from 0.154.0 to 0.155.1 using the actual Rust update implementation; the post-update launch-shell probe confirmed 0.155.1.
- Claude Code and Gemini were detected in their fnm/npm installation; Hermes was detected through its native launcher. No updates were performed for those tools.
- The initial OpenCode pnpm validation covered source detection only. The subsequent user upgrade exposed a skipped postinstall and a broken executable. Regression coverage now includes package-scoped build permission, failed-probe diagnostics (including misleading version strings in errors), and the repair UI. OpenClaw was not installed.
- The corrected Rust repair path was executed against that broken local installation with pnpm 11.19.0: OpenCode's postinstall completed, the result was `repaired`, and an independent login-shell probe returned **1.18.31**. Registry latest was **1.18.32**, published less than one day earlier; pnpm's default release-age policy was not bypassed. This verifies repair and execution, not installation of 1.18.32 or a clean-VM acceptance.
- Homebrew execution, native Hermes/Claude/OpenCode execution, and Windows detection have automated coverage where applicable but were not manually validated on clean VMs. This is not a release certification.

### ChatGPT desktop (2026-09-30)

- macOS 15.6.1 ARM64 VM, ChatGPT 26.924.22138 in `~/Applications`: `verify_chatgpt_install --check-latest` reported latest **26.928.21956**, `update_status: available`, `update_source: sparkle`, `update_supported: false`; `--running` reported `false`. The live-feed test parsed arm64 26.928.21956, x64 26.928.21956 and Windows 26.928.2636.0.
- The Windows upgrade path has unit coverage for its pure parts (winget exit codes, package-path matching, process lists) but has not been compiled locally or run on a Windows machine yet; rely on the Windows Probe workflow and a manual VM run before release.
