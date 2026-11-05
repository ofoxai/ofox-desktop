import { writeFile, chmod, mkdir, rename } from "node:fs/promises";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { mcpScrubber, mcpSecrets } from "./mcp-security.mjs";

export async function writeSessionMcpConfig(agentDir, mcp) {
  const cleaner = mcpScrubber(mcpSecrets(mcp));
  const servers = Object.create(null),
    unavailable = [];
  for (const [name, config] of Object.entries(mcp?.servers ?? {})) {
    if (config.enabled === false) continue;
    if (cleaner.text(name) !== name) {
      unavailable.push({ name: null, reason: "unavailable" });
      continue;
    }
    if (config.unavailable === true) {
      unavailable.push({ name, reason: "unavailable" });
      continue;
    }
    if (config.command === "npx" && !mcp.npxPath) {
      unavailable.push({ name, reason: "needs_node" });
      continue;
    }
    if (config.command === "uvx" && !mcp.uvxPath && !mcp.uvAvailable) {
      unavailable.push({ name, reason: "needs_uv" });
      continue;
    }
    const server = { ...config, exposure: "deferred" };
    delete server.toolExposure;
    delete server.auth;
    delete server.oauth;
    if (config.command === "npx") server.command = mcp.npxPath;
    if (config.command === "uvx" && mcp.uvxPath) server.command = mcp.uvxPath;
    if (
      mcp?.npmRegistry &&
      config.env?.npm_config_registry === undefined &&
      config.env?.NPM_CONFIG_REGISTRY === undefined
    )
      server.env = { ...config.env, npm_config_registry: mcp.npmRegistry };
    servers[name] = server;
  }
  await mkdir(agentDir, { recursive: true, mode: 0o700 });
  await chmod(agentDir, 0o700);
  const file = path.join(agentDir, "mcp.json");
  const temporary = `${file}.${randomUUID()}.tmp`;
  await writeFile(
    temporary,
    JSON.stringify({
      autoEnableCodemode: false,
      mcpServers: cleaner.scrub(servers),
    }),
    { mode: 0o600, flag: "wx" },
  );
  await rename(temporary, file);
  return {
    written: Object.keys(servers),
    unavailable: cleaner.scrub(unavailable),
    servers,
  };
}
