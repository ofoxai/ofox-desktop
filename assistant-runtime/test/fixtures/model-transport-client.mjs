// A separate process trusts only the explicitly supplied synthetic test CA;
// the test runner's TLS trust, environment and global dispatcher stay intact.
import { modelTransport } from "../../src/model-transport.mjs";

const { url, proxy, nativeRequest, abortAfterMs, cancelAfterMs } = JSON.parse(
  process.argv[2],
);
const transport = await modelTransport(proxy);
const request = transport?.fetch ?? globalThis.fetch;
const controller = new AbortController();
const cancel = cancelAfterMs
  ? setTimeout(() => controller.abort(), cancelAfterMs)
  : undefined;
const started = performance.now();
const result = {};
try {
  const init = {
    signal: AbortSignal.any([
      controller.signal,
      AbortSignal.timeout(abortAfterMs ?? 25000),
    ]),
    headers: { authorization: "Bearer synthetic-transport-test" },
  };
  const response = nativeRequest
    ? await request(new Request(url, init))
    : await request(url, init);
  result.status = response.status;
  result.body = await response.json();
} catch (error) {
  result.errorName = error.name;
  result.errorCode = error.code;
  result.causeCode = error.cause?.code;
} finally {
  clearTimeout(cancel);
  result.elapsedMs = Math.round(performance.now() - started);
  await transport?.close();
}
if (transport) {
  try {
    await request(url);
    result.closedBlocksRequests = false;
  } catch (error) {
    result.closedBlocksRequests = error.cause?.code === "UND_ERR_DESTROYED";
  }
}
console.log(JSON.stringify(result));
