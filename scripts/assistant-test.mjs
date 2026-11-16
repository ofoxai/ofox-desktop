import { execFileSync, spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
execFileSync(
  process.execPath,
  [path.join(root, "scripts/assistant-prepare.mjs")],
  { stdio: "inherit" },
);
const resources = path.join(root, "assistant-runtime/resources");
const manifest = JSON.parse(
  readFileSync(path.join(resources, "manifest.json"), "utf8"),
);
const node = path.join(resources, manifest.node);
const run = (args, directory) =>
  spawnSync(node, args, {
    cwd: path.join(root, directory),
    stdio: "inherit",
  }).status === 0;

// The runtime's tests, then the search server's (proxy, TLS name checks, the
// notice of what its bundle compiles in) on the same bundled Node.
const failed = [];
if (!run(["--test", "test/*.test.mjs"], "assistant-runtime"))
  failed.push("assistant runtime tests");
const search = "assistant-runtime/search-mcp";
if (!existsSync(path.join(root, search, "node_modules")))
  failed.push(`${search} has no dependencies: run pnpm assistant:install`);
else if (!run(["build.mjs"], search)) failed.push("search server build");
else if (!run(["--test", "test/*.test.mjs"], search))
  failed.push("search server tests");
if (failed.length) {
  console.error(`Assistant tests failed: ${failed.join("; ")}`);
  process.exit(1);
}
