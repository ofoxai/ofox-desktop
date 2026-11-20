let nextOpening = 0;

// Give an existing tool scenario its own admission round without altering
// the receipts for the operation(s) the scenario is testing.
export function planningOpening(kind = "plan") {
  const name = kind === "single" ? "beginSingleStep" : "updatePlan";
  const args =
    kind === "single"
      ? {}
      : {
          steps: [
            { step: "Run the fixture operations", status: "in_progress" },
            { step: "Report their outcomes", status: "pending" },
          ],
        };
  const id = `fixture-admission-${++nextOpening}`;
  const frames = [
    {
      choices: [
        {
          index: 0,
          delta: {
            role: "assistant",
            tool_calls: [
              {
                index: 0,
                id,
                type: "function",
                function: { name, arguments: JSON.stringify(args) },
              },
            ],
          },
        },
      ],
    },
    {
      choices: [{ index: 0, delta: {}, finish_reason: "tool_calls" }],
      usage: { prompt_tokens: 1, completion_tokens: 1, total_tokens: 2 },
    },
  ];
  return new Response(
    frames.map((frame) => `data: ${JSON.stringify(frame)}\n\n`).join("") +
      "data: [DONE]\n\n",
    { headers: { "content-type": "text/event-stream" } },
  );
}

/** Keep a fixture's existing script and request list focused on its tools. */
export function withPlanningOpening(fetch, kind = "plan") {
  return async (url, options) => {
    const body = JSON.parse(options.body);
    const names = (body.tools ?? []).map(
      (tool) => tool.function?.name ?? tool.name,
    );
    const messages = body.messages ?? [];
    const latestUser = messages.findLastIndex(
      (message) => message.role === "user",
    );
    const current = messages.slice(latestUser + 1);
    if (
      names.length === 2 &&
      names.includes("beginSingleStep") &&
      names.includes("updatePlan") &&
      !current.some((message) => isFixtureOpening(message.tool_call_id))
    )
      return planningOpening(kind);
    return fetch(url, options);
  };
}

export const isFixtureOpening = (id) =>
  typeof id === "string" && id.startsWith("fixture-admission-");
