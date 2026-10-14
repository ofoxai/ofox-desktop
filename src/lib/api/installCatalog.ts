import { invoke } from "@tauri-apps/api/core";
import type { InstallationStatus } from "./toolUpdates";

export interface InstallInventoryEntry {
  toolId: string;
  status: InstallationStatus;
  version?: string | null;
  path?: string | null;
  error?: string | null;
}

/** Local probes only: no OFox keys, bindings, model snapshots or latest-version HTTP. */
export async function getInstallInventory(): Promise<InstallInventoryEntry[]> {
  const rows = await invoke<InstallInventoryEntry[]>(
    "get_install_catalog_inventory",
  );
  if (!Array.isArray(rows))
    throw new Error("Installation inventory is unavailable");
  return rows;
}
