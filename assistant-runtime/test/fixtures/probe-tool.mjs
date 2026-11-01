import { Type } from "typebox";

// S1 ships no model tools. Tests register this host-backed probe through the
// runtime's own wrapper so the tool-call budget, identity, deduplication and
// unknown-outcome gate that later tools reuse stay covered.
export const probeToolName = "probe_write";
export const probeGuideline = "Ask before every probe write.";

export function probeTools(runtime, context) {
  return [
    {
      ...runtime.wrapTool(
        context,
        probeToolName,
        "Test-only host operation with side effects.",
        Type.Object({ command: Type.String({ minLength: 1, maxLength: 200 }) }),
        (args, signal, toolCallId) =>
          runtime.hostTool(context, probeToolName, args, signal, toolCallId),
      ),
      promptGuidelines: [probeGuideline],
    },
  ];
}
