/** Benchmark-only boundary: no native commands or external network requests. */
export const blocked = { ipc: 0, network: 0 };
export async function invoke(command: string): Promise<unknown> {
  if (command === "ofox_get_apex_state")
    return { apex: "ofox.ai", pinned: true };
  blocked.ipc++;
  throw new Error(`Benchmark disables native IPC: ${command}`);
}
export async function listen(): Promise<() => void> {
  return () => undefined;
}
export async function save(): Promise<unknown> {
  return invoke("save_dialog");
}
