// The Office reader's own process (see readOffice in office.mjs). It reads
// only the bytes it is sent and answers once.
import v8 from "node:v8";
import { officeText } from "./office-core.mjs";
import { OfficeError } from "./office.mjs";

process.once("message", async ({ bytes, extension, deadline }) => {
  let reply;
  try {
    const result = await officeText(bytes, extension, { deadline });
    reply = {
      result: { ...result, heapLimit: v8.getHeapStatistics().heap_size_limit },
    };
  } catch (error) {
    // Never forward parser messages: they can quote the user's XML.
    reply = {
      reason: error instanceof OfficeError ? error.reason : "invalid_office",
    };
  }
  process.send(reply, () => process.disconnect());
});
// Only now that its modules have loaded: the parent sends the bytes.
process.send({ ready: true });
