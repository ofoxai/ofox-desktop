import { invoke } from "@tauri-apps/api/core";

export type AgentDirTool = "codex" | "claude";

/**
 * What decided the directory, in order of precedence: the directory typed into
 * Ofox's settings, the `envVar` in Ofox's own environment, the `envVar` in the
 * user's login shell (Windows: the registry's environment), the default in the home.
 */
export type AgentDirSource = "settings" | "env" | "loginShell" | "default";

export interface AgentDir {
  tool: AgentDirTool;
  /** Where the agent keeps its configuration, sessions and skills. */
  dir: string;
  source: AgentDirSource;
  /** The variable the agent reads to move this directory. */
  envVar: string;
}

/** Where Codex and Claude Code keep their files, and what decided it. */
export async function getAgentDirs(): Promise<AgentDir[]> {
  return await invoke("get_agent_dirs");
}
