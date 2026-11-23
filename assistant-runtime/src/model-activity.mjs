// Liveness without model text: include bytes held back by secret scrubbing and
// streamed tool arguments, but never let empty SDK/status events renew a run.
export function modelActivity(notify, now = () => performance.now()) {
  let bytes = 0;
  let sentAt = -Infinity;
  return (delta) => {
    if (typeof delta !== "string" || delta.length === 0) return;
    bytes += Buffer.byteLength(delta, "utf8");
    const at = now();
    if (at - sentAt >= 1000) {
      sentAt = at;
      notify(bytes);
    }
  };
}
