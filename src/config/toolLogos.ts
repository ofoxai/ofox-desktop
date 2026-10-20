import claude from "@/assets/agent-logos/claudecode-color.svg?url";
import codex from "@/assets/agent-logos/codex-color.svg?url";
import gemini from "@/assets/agent-logos/geminicli-color.svg?url";
import hermes from "@/assets/agent-logos/hermes.svg?url";
import cline from "@/assets/agent-logos/cline.svg?url";
import crush from "@/assets/agent-logos/crush.png";
import omp from "@/assets/agent-logos/omp.svg?url";
import codebuddy from "@/assets/agent-logos/codebuddy-color.svg?url";
import devecoCode from "@/assets/agent-logos/deveco-code.png";
import devecoCli from "@/assets/agent-logos/deveco-cli.png";
import aider from "@/assets/agent-logos/aider.png";
import goose from "@/assets/agent-logos/goose.svg?url";
import chatgpt from "@/icons/extracted/openai.svg?url";
import opencode from "@/icons/extracted/opencode-logo-light.svg?url";
import openclaw from "@/icons/extracted/claw.svg?url";
import workbuddy from "@/icons/extracted/workbuddy.svg?url";
import dsh from "@/icons/extracted/deepseek.svg?url";
import pi from "@/icons/extracted/pi.svg?url";
import copilot from "@/icons/extracted/githubcopilot.svg?url";
import cursor from "@lobehub/icons-static-svg/icons/cursor.svg?url";
import claudeDesktop from "@lobehub/icons-static-svg/icons/claude-color.svg?url";

interface ToolLogo {
  src: string;
  label: string;
  darkTreatment?: "white" | "invert";
}

/** Bundled product marks; provenance is recorded in docs/brand-assets.md. */
export const TOOL_LOGOS: Record<string, ToolLogo> = {
  claude: { src: claude, label: "Claude Code" },
  codex: { src: codex, label: "Codex" },
  chatgpt: { src: chatgpt, label: "ChatGPT", darkTreatment: "white" },
  gemini: { src: gemini, label: "Gemini CLI" },
  opencode: { src: opencode, label: "OpenCode", darkTreatment: "invert" },
  openclaw: { src: openclaw, label: "OpenClaw" },
  workbuddy: { src: workbuddy, label: "WorkBuddy" },
  dsh: { src: dsh, label: "DeepSeek Harness" },
  pi: { src: pi, label: "Pi", darkTreatment: "white" },
  hermes: { src: hermes, label: "Hermes", darkTreatment: "white" },
  copilot: {
    src: copilot,
    label: "GitHub Copilot CLI",
    darkTreatment: "white",
  },
  cline: { src: cline, label: "Cline CLI", darkTreatment: "white" },
  crush: { src: crush, label: "Crush" },
  omp: { src: omp, label: "oh-my-pi (omp)", darkTreatment: "white" },
  codebuddy: { src: codebuddy, label: "CodeBuddy Code CLI" },
  "deveco-code": { src: devecoCode, label: "DevEco Code" },
  "deveco-cli": { src: devecoCli, label: "DevEco CLI" },
  aider: { src: aider, label: "Aider" },
  "goose-cli": { src: goose, label: "Goose CLI", darkTreatment: "white" },
  // Library skill targets that are not in the installation catalog.
  cursor: { src: cursor, label: "Cursor", darkTreatment: "white" },
  "claude-desktop": { src: claudeDesktop, label: "Claude Desktop" },
};
