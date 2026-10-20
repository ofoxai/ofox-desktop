import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

/** The main window opens its settings on this (ConsolePage listens). */
export const OFOX_OPEN_SETTINGS_EVENT = "ofox-open-settings";
/** The main window goes to one-click binding on this (MainApp listens). */
export const OFOX_START_BINDING_EVENT = "ofox-start-binding";

/** Steps the popover out of the way; losing focus hides it anyway. */
export async function hidePopover() {
  try {
    await getCurrentWindow().hide();
  } catch (e) {
    console.error("[tray] hide popover failed", e);
  }
}

/** Brings the main window up, with a request for it if any, and hides the popover. */
export async function openMainWindow(request?: string) {
  await invoke("show_main_window");
  if (request) await emit(request);
  await hidePopover();
}
