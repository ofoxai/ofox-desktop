import { invoke } from "@tauri-apps/api/core";

/** 装 npm 工具前，本机 Node.js 是否满足它最新版的要求（见 `node_requirement.rs`）。 */
export interface NodeRequirement {
  /** `missing` 时安装器会自动装 Node；`unknown` 查不到，照常安装。 */
  status: "ok" | "missing" | "tooOld" | "unknown" | "notApplicable";
  required: string | null;
  current: string | null;
  /** fnm / nvm / nvm-windows / volta / homebrew / nodejs / asdf / mise / unknown */
  manager: string | null;
  canUpgrade: boolean;
  needsAdmin: boolean;
  /** 升级后要重新安装的工具 id。 */
  reinstall: string[];
  /** 用户自己升级的命令或做法。 */
  manual: string | null;
  /** 至少要升到的版本（例如 `24.16.0`）；看不懂范围时为空。 */
  minimum: string | null;
  /** 同意后终端里运行的完整命令（含重装工具）；Ofox 升级不了时为空。 */
  command: string | null;
}

export function checkToolNodeRequirement(
  tool: string,
): Promise<NodeRequirement> {
  return invoke<NodeRequirement>("check_tool_node_requirement", { tool });
}
