import { writeSync } from "node:fs";

export const traceRuntime = process.argv[2] === "--ofox-runtime-diagnostics";

export function markStage(stage) {
  if (!traceRuntime) return;
  try {
    // Only fixed milestones belong here. Flush even on immediate exit.
    writeSync(2, `OFOX_RUNTIME_STAGE:${stage}\n`);
  } catch {
    // Losing the diagnostic pipe must not fail a run.
  }
}

// Imported first by main.mjs to mark entry before the runtime evaluates.
markStage("entry");
