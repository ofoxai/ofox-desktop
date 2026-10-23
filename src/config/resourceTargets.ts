import { TOOL_ORDER } from "./toolMeta";

/** Codex and ChatGPT use the same native configuration and ownership receipts. */
export const resourceTargetId = (client: string) =>
  client === "chatgpt" ? "codex" : client;
export const RESOURCE_TARGET_IDS = [
  ...new Set(TOOL_ORDER.map(resourceTargetId)),
];
