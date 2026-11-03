// Input lines above this are never parsed. The host keeps tool replies below
// it; a larger reply still settles its pending call by id instead of leaving
// the tool waiting until the run deadline.
export const LINE_LIMIT = 1024 * 1024;

export function oversizedReplyId(line) {
  const match = /"id"\s*:\s*"(tool-[A-Za-z0-9-]{1,80})"/.exec(
    line.slice(0, 512),
  );
  return match?.[1] ?? null;
}
