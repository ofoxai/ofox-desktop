export interface ShowcaseDiagnostics {
  blockedIpc: string[];
  blockedNetwork: string[];
  calls: string[];
  ready: boolean;
}
export function installReadmeSandbox(
  target: Window,
  dispatch: (command: string, payload: Record<string, unknown>) => unknown,
): ShowcaseDiagnostics;
