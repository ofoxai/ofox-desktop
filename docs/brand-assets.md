# Agent product marks

The installation catalog uses a bundled product image for each of its 19 IDs.
`src/config/toolLogos.ts` is the shared ToolBadge mapping. Images do not require
runtime network requests. Monochrome marks receive a dark-theme treatment; OpenCode uses inversion
that preserves its internal grayscale, while colored marks retain their colors. Provider icons remain a separate
concept from agent product icons.

Sources checked on 2026-10-15:

| Tool               | Bundled image and source                                                                                                                                                                                                         |
| ------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Claude Code        | `agent-logos/claudecode-color.svg`, Magpie `bfaedf18`                                                                                                                                                                            |
| Codex              | `agent-logos/codex-color.svg`, Magpie `bfaedf18`; distinct from ChatGPT                                                                                                                                                          |
| ChatGPT            | Existing `icons/extracted/openai.svg`, OpenAI mark from Lobe Icons                                                                                                                                                               |
| Gemini CLI         | `agent-logos/geminicli-color.svg`, Magpie `bfaedf18`                                                                                                                                                                             |
| OpenCode           | Existing `icons/extracted/opencode-logo-light.svg`, introduced in `cb1b45ae`; exact upstream image URL not reverified in this pass                                                                                               |
| OpenClaw           | Existing static `icons/extracted/claw.svg`; geometry checked against [official favicon](https://raw.githubusercontent.com/openclaw/openclaw/main/ui/public/favicon.svg), whose current version also animates                     |
| WorkBuddy          | Existing `icons/extracted/workbuddy.svg`, introduced in `6e051d34`; exact upstream image URL not reverified in this pass                                                                                                         |
| DeepSeek Harness   | Existing `icons/extracted/deepseek.svg`, identical to [Lobe Icons deepseek-color.svg](https://raw.githubusercontent.com/lobehub/lobe-icons/master/packages/static-svg/icons/deepseek-color.svg)                                  |
| Pi                 | Existing `icons/extracted/pi.svg`; [official product](https://pi.dev/)                                                                                                                                                           |
| Hermes             | `agent-logos/hermes.svg`, Magpie `bfaedf18` product mark, replacing the previous avatar in ToolBadge                                                                                                                             |
| GitHub Copilot CLI | Existing `icons/extracted/githubcopilot.svg`, Lobe Icons; the Microsoft Copilot mark is not used                                                                                                                                 |
| Cline CLI          | `agent-logos/cline.svg`, installed Lobe Icons `1.73.0`                                                                                                                                                                           |
| Crush              | `agent-logos/crush.png`, Magpie `bfaedf18`; [official brand image](https://stuff.charm.sh/crush/charm-crush.png)                                                                                                                 |
| oh-my-pi           | `agent-logos/omp.svg`, Magpie `bfaedf18`; this records the selected asset, not a claim that it is the latest branding                                                                                                            |
| CodeBuddy Code CLI | `agent-logos/codebuddy-color.svg`, Magpie `bfaedf18` / Lobe Icons                                                                                                                                                                |
| DevEco Code        | `agent-logos/deveco-code.png`, [Huawei product image](https://developer.huawei.com/allianceCmsResource/resource/HUAWEI_Developer_VUE/images/DevEcocode-1784601043696.png)                                                        |
| DevEco CLI         | `agent-logos/deveco-cli.png`, [Huawei CLI image](https://developer.huawei.com/allianceCmsResource/resource/HUAWEI_Developer_VUE/images/DevEcocli-1784601095538.png); distinct from DevEco Code                                   |
| Aider              | `agent-logos/aider.png`, [official site image](https://aider.chat/assets/icons/android-chrome-384x384.png), confirmed in [the project's brand discussion](https://github.com/Aider-AI/aider/issues/1100#issuecomment-2296194446) |
| Goose CLI          | `agent-logos/goose.svg`, installed Lobe Icons `1.73.0`                                                                                                                                                                           |

Magpie assets are copied from the same `bfaedf18` reference used by the design.
The copied files retain the Magpie MIT notice (Copyright 2026 yetone), and
Lobe Icons assets retain its MIT notice (Copyright 2023 LobeHub). The notices
and license texts are in `src/assets/agent-logos/`. Aider's Apache-2.0 and
Crush's current FSL-1.1-MIT texts are preserved separately; no blanket MIT
claim is made for all upstream material. The two Huawei images are official
product material, without asserting a separate open-source image license.
Brand marks remain the property of their respective owners.
