# Manual installation commands

The optional catalog includes unbound core CLIs as well as additional products. Core entries must include their manual installation recipe; inheriting a name and project link from `TOOL_ORDER` alone leaves an empty command column. Installed rows keep their existing inventory presentation. Displaying or copying a recipe never executes it or changes bindings.

Commands are selected for the current desktop platform. Unknown hosts receive a guide hint instead of a guessed shell command. ChatGPT and WorkBuddy are desktop downloads and show an explicit download hint. Official links remain available in every case, including an older backend without inventory support.

Sources checked on 2026-10-16:

| Tool | Manual recipe / source |
| --- | --- |
| Pi | `npm install -g --ignore-scripts @earendil-works/pi-coding-agent`; Node.js 22.19+. [Official README](https://github.com/earendil-works/pi/blob/main/packages/coding-agent/README.md). The old `@mariozechner` scope and separate `@oh-my-pi` fork are not substituted. |
| Goose CLI | macOS: `brew install block-goose-cli`; Linux: the official Bash download script; Windows: download and run the official PowerShell script as two separate lines. [Official installation](https://github.com/aaif-goose/goose/blob/main/documentation/docs/getting-started/installation.md). The unrelated `goose` Homebrew formula is not used. |
| Claude Code | Official Bash / native PowerShell installer. [Setup](https://code.claude.com/docs/en/setup). |
| OpenClaw | Official Bash / native PowerShell installer, which handles runtime requirements. [Installation](https://docs.openclaw.ai/install). A fixed npm recipe would otherwise depend on npm lifecycle-script policy. |
| Hermes | Official Bash / native PowerShell installer. [Installation](https://hermes-agent.nousresearch.com/docs/getting-started/installation). |
| Codex | `npm install -g @openai/codex@latest`. [Official documentation](https://developers.openai.com/cookbook/examples/codex/using_goals_in_codex). |
| OpenCode | `npm install -g opencode-ai`, matching this project's stable v1 integration. [Official documentation](https://opencode.ai/docs/). The v2 preview package is not substituted. |
| Gemini | `npm install -g @google/gemini-cli`. [Official installation](https://geminicli.com/docs/get-started/installation/). |
| dsh | `npm install -g @deepseek-ai/dsh`, matching OFox's existing installer package and the upstream package's CLI entry. The upstream [quickstart](https://github.com/deepseek-ai/deepseek-harness) uses `npx … web`, which starts the tool; it is not presented as a global installation command. |

These are documentation-checked copyable instructions. This fix does not install any candidate tool, execute third-party scripts, or validate Windows installation behavior. Native Windows remains a separate acceptance gate. Shell scripts may provision dependencies; runtime prerequisites and vendor changes remain governed by each official guide.
