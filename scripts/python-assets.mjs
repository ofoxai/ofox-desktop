// Pyodide's own wheels are pinned at the canonical jsDelivr URL in
// python-downloads.json. jsDelivr's main host can be very slow from some
// networks, so OFOX_PYODIDE_BASE_URL replaces that base when downloading.
// Only the base changes: assistant-prepare.mjs still verifies every file
// against its pinned SHA-256.
export const PYODIDE_BASE_URL =
  "https://cdn.jsdelivr.net/pyodide/v314.0.7/full/";

export function pyodideWheelUrl(pin, env = process.env) {
  const base = env.OFOX_PYODIDE_BASE_URL;
  if (!base) return pin.url;
  return `${base.endsWith("/") ? base : `${base}/`}${pin.file}`;
}

// The Python engine is offered only on the platforms python-downloads.json
// lists (macOS). Everywhere else its wheels are not staged and the pyodide npm
// package, which can only run that engine, is pruned from the production tree.
export const pythonEngineOffered = (platform, downloads) =>
  downloads.platforms.includes(platform);
