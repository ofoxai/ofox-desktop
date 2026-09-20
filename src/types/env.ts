/**
 * 环境变量冲突检测相关类型定义
 */

/**
 * 环境变量冲突信息
 */
export interface EnvConflict {
  /** 环境变量名称 */
  varName: string;
  /** 环境变量的值 */
  varValue: string;
  /** 来源类型: "system" 表示系统环境变量, "file" 表示配置文件 */
  sourceType: "system" | "file";
  /** 来源路径（注册表路径或文件路径；新记录不再拼接行号） */
  sourcePath: string;
  /** 配置文件中的 1-based 行号 */
  lineNumber?: number;
  /** 原生系统或用户显式配置的 WSL 环境 */
  environment: "native" | "wsl";
  /** environment 为 wsl 时的发行版名称 */
  wslDistro?: string;
}

export const envConflictKey = (conflict: EnvConflict): string =>
  JSON.stringify([
    conflict.environment,
    conflict.wslDistro ?? "",
    conflict.varName,
    conflict.sourcePath,
    conflict.lineNumber ?? null,
  ]);

/**
 * 备份信息
 */
export interface BackupInfo {
  /** 备份文件路径 */
  backupPath: string;
  /** 备份时间戳 */
  timestamp: string;
  /** 被备份的环境变量冲突列表 */
  conflicts: EnvConflict[];
}
