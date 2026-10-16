/** Benchmark-only boundary: no native commands or external network requests. */
export const blocked = { ipc: 0, network: 0 };
export async function invoke(command: string): Promise<never> {
  blocked.ipc++;
  throw new Error(`Benchmark disables native IPC: ${command}`);
}
export async function listen(): Promise<() => void> {
  return () => undefined;
}
export async function save(): Promise<never> {
  return invoke("save_dialog");
}
