// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { ASSISTANT_ERROR_MESSAGES } from "@/lib/api/assistant";

const SOURCES = [
  ...readdirSync(
    new URL("../../src-tauri/src/services/assistant/", import.meta.url),
    { recursive: true, encoding: "utf8" },
  )
    .filter((name) => name.endsWith(".rs"))
    .map((name) => `../../src-tauri/src/services/assistant/${name}`),
  "../../src-tauri/src/commands/assistant.rs",
  "../../src-tauri/src/commands/assistant_local.rs",
];

/** Codes the host uses only inside itself; none reaches the renderer. */
const HOST_ONLY = new Set([
  // A refused key is retried with a new one inside the run.
  "assistant_retry_with_new_key",
  // The host's reply to a runtime tool call.
  "assistant_unknown_tool",
]);

/** Codes the renderer raises itself. */
const RENDERER_CODES = [
  "assistant_microphone_denied",
  "assistant_microphone_unavailable",
  "assistant_recording_interrupted",
  "assistant_audio_empty",
  "assistant_session_loading",
  "assistant_request_expired",
];

/** Every `assistant_*` string literal outside the test modules. */
function backendCodes() {
  const codes = new Set<string>();
  for (const path of SOURCES) {
    const source = readFileSync(new URL(path, import.meta.url), "utf8");
    const production = source.split("#[cfg(test)]")[0];
    for (const [, code] of production.matchAll(/"(assistant_[a-z0-9_]+)"/g))
      if (!HOST_ONLY.has(code)) codes.add(code);
  }
  return codes;
}

describe("assistant error code parity", () => {
  it("maps exactly the codes the backend can send, plus the renderer's own", () => {
    const backend = backendCodes();
    expect(backend.size).toBeGreaterThan(40);
    expect(Object.keys(ASSISTANT_ERROR_MESSAGES).sort()).toEqual(
      [...backend, ...RENDERER_CODES].sort(),
    );
  });
});
