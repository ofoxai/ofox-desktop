export interface InstallStamp {
  lockSha256: string;
  packageSha256: string;
  scriptsSha256: string;
  nodeVersion: string;
  platform: string;
}
export function installStamp(options: {
  runtime: string;
  scripts: string[];
  nodeVersion: string;
  platform: string;
}): Promise<InstallStamp>;
export function installIsCurrent(
  production: string,
  stampFile: string,
  wanted: InstallStamp,
): Promise<boolean>;
export function writeInstallStamp(
  stampFile: string,
  stamp: InstallStamp,
): Promise<void>;
export function clearInstallStamp(stampFile: string): Promise<void>;
