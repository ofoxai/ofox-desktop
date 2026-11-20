// Loaded with `node --import` into a test sidecar only: main.mjs imports the
// same runtime module instance, so its host bridge serves the probe tool.
import { AssistantRuntime } from "../../src/runtime.mjs";
import { probeTools } from "./probe-tool.mjs";

const originalTools = AssistantRuntime.prototype.tools;
AssistantRuntime.prototype.tools = function (context, model) {
  return [
    ...originalTools.call(this, context, model),
    ...probeTools(this, context),
  ];
};
