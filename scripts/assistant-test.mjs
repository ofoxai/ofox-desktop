import { execFileSync, spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const output = execFileSync(
  process.execPath,
  [path.join(root, "scripts/assistant-prepare.mjs"), "--node-only"],
  { encoding: "utf8" },
);
const node = output.trim().split(/\r?\n/).at(-1);
const result = spawnSync(node, ["--test", "test/*.test.mjs"], {
  cwd: path.join(root, "assistant-runtime"),
  stdio: "inherit",
});
process.exit(result.status ?? 1);
