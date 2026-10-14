# Lessons from repository history

These are verified commit references, not claims that every external agent version was tested.

| Rule | Why and evidence |
| --- | --- |
| Probe the installed agent before writing version-sensitive keys. | Removing `disable_response_storage` globally broke versions that accept it; unconditional restoration broke others. `e25d5adc`, revert `ad2d5062`, and capability-aware fixes `79da68b0` / `38a681f5` document the change. A TOML parser accepting a key does not establish agent compatibility. |
| Follow model precedence through every sibling integration. | CLI/GUI remembered selections can outrank, or be overridden by, settings and environment. Model fixes `c09bae5b`, `65179dbb`, `eedc53e8`, `55550f32`, `671ed1f3` show why a one-agent fix is incomplete. Preserve user effort and distinguish observed from managed selection. |
| Run all-targets Clippy and tests on each supported OS. | Windows-only branches were invisible to macOS checks; `e9556dc1` and `fadc150b` required Windows fixes. Cross-platform type checking is insufficient. |
| Exercise release workflows before release day, against the same commit. | `97935497` records a release workflow repair. Full reusable CI and a successful exact-SHA Windows Probe must precede builds, even for dry-run. Never substitute a green result from another branch head. |
| Unreadable settings are not empty settings. | `28c46c15` added backup recovery and blocked writes when parsing fails; `73569a64` exposed health and explicit reset. Recovery must preserve evidence, surface health on initial mount, and require confirmation before defaults replace unreadable settings. |
| Use one serial lock for shared settings/environment tests. | `eebf3d0d` fixed cross-test interference between uninstall sandboxes and settings/WebDAV tests. A private lock name only serializes its own group. |
| Bind asynchronous work to account and component lifetime. | `1559001a`, `a5cd8c6c`, and `b840f7d1` cover abandoned account responses, continuous balance samples and delayed job subscriptions. Subscribe before starting jobs, validate operation IDs, and prevent stale completion callbacks. |

See [development standards](docs/code-standards.md) and [integration wiring](docs/agent-wiring.md).
