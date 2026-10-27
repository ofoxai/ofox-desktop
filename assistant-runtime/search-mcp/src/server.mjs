// Ofox search MCP server over stdio. stdout carries only MCP messages; stderr
// gets short redacted diagnostics, because the host shows its tail to users.
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { loadConfig } from "./config.mjs";
import { redact } from "./redact.mjs";
import { createSearchServer } from "./tools.mjs";

const secrets = [process.env.OFOX_ASSISTANT_KEY];

function report(error) {
  const message = error instanceof Error ? error.message : String(error);
  process.stderr.write(
    `ofox-search-mcp: ${redact(message, secrets).slice(0, 300)}\n`,
  );
}

process.on("uncaughtException", (error) => {
  report(error);
  process.exit(1);
});
process.on("unhandledRejection", (error) => {
  report(error);
  process.exit(1);
});

let config;
try {
  config = loadConfig(process.env);
} catch (error) {
  report(error);
  process.exit(2);
}

const server = createSearchServer(config);
server.server.onerror = report;
await server.connect(new StdioServerTransport());
