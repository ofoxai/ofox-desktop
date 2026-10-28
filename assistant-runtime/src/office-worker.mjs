import { parentPort, workerData } from "node:worker_threads";
import { officeText } from "./office-core.mjs";
import { OfficeError } from "./office.mjs";

try {
  parentPort.postMessage({
    result: await officeText(workerData.bytes, workerData.extension),
  });
} catch (error) {
  // Never forward parser errors containing fragments of the user's XML.
  parentPort.postMessage({
    reason:
      error instanceof OfficeError
        ? error.reason
        : /Maximum nested tags/.test(error.message)
          ? "office_expansion"
          : "invalid_office",
  });
}
