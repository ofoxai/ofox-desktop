// Keep one SDK message lifecycle when a delivered document's empty failed
// closeout is replaced. No raw HTTP/SSE lookahead and no tool/turn replay.
export async function forwardModelStream(attempt, { push, recover }) {
  if (!recover) {
    for await (const event of attempt.source) push(event);
    return attempt.source.result();
  }
  for (;;) {
    let start, terminal;
    let active = false;
    const flushStart = () => {
      if (start) {
        push(start);
        start = undefined;
      }
    };
    for await (const event of attempt.source) {
      if (event.type === "start") start = event;
      else if (event.type === "done" || event.type === "error")
        terminal = event;
      else {
        // Unknown future SDK events also make recovery ineligible.
        active = true;
        flushStart();
        push(event);
      }
    }
    const result = await attempt.source.result();
    const replacement = !active && (await recover(result, attempt));
    if (replacement) {
      attempt = replacement;
      continue;
    }
    flushStart();
    if (terminal) push(terminal);
    return result;
  }
}
