// CI-only developer acceptance artifact. Never executes a model task.
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { lstat, mkdir, readFile, realpath, writeFile } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const root = path.resolve(fileURLToPath(new URL("..", import.meta.url)));
const binaryName = "assistant_host-parity.exe";
const entryPoint = "parity_eval::one_guarded_task";
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
// Git's environment overrides can redirect both status and build.rs to another
// worktree/index even when cwd is correct. Use the actual checkout for both.
export const sourceEnvironment = (environment = process.env) => ({
  ...Object.fromEntries(
    Object.entries(environment).filter(
      ([name]) => !name.toUpperCase().startsWith("GIT_"),
    ),
  ),
  GIT_CONFIG_NOSYSTEM: "1",
  GIT_CONFIG_GLOBAL: os.devNull,
});
const git = (checkout, args) =>
  execFileSync("git", args, {
    cwd: checkout,
    encoding: "utf8",
    env: sourceEnvironment(),
  });

export function assertCleanCheckout(checkout, expectedSha) {
  assert.match(expectedSha, /^[a-f0-9]{40}$/);
  assert.equal(
    git(checkout, ["rev-parse", "HEAD"]).trim(),
    expectedSha,
    "driver checkout HEAD differs from expected SHA",
  );
  assert.equal(
    git(checkout, ["status", "--porcelain=v1", "--untracked-files=normal"]),
    "",
    "driver compilation requires a clean checkout, including untracked files",
  );
}

export function selectCargoDriver(output, expectedSha) {
  assert.match(expectedSha, /^[a-f0-9]{40}$/);
  const messages = output
    .split(/\r?\n/)
    .filter((line) => line.trim())
    .map((line) => JSON.parse(line));
  const completed = messages.filter(
    (message) => message.reason === "build-finished",
  );
  assert.ok(
    completed.length === 1 && completed[0].success === true,
    "Cargo did not report one successful complete build",
  );
  const artifacts = messages.filter(
    (message) =>
      message.reason === "compiler-artifact" &&
      message.target?.name === "assistant_host" &&
      message.target?.kind?.includes("test") &&
      message.profile?.test === true &&
      typeof message.executable === "string",
  );
  assert.equal(
    artifacts.length,
    1,
    "expected exactly one assistant_host test executable",
  );
  const artifact = artifacts[0];
  assert.match(
    path.posix.basename(path.win32.basename(artifact.executable)),
    /^assistant_host-[a-f0-9]+\.exe$/,
    "Cargo did not identify the Windows integration test executable",
  );
  // The repository's [profile.release] optimizes for size.
  assert.equal(
    artifact.profile.opt_level,
    "s",
    "driver must use the configured release profile",
  );
  assert.equal(artifact.profile.debug_assertions, false);
  const scripts = messages.filter(
    (message) =>
      message.reason === "build-script-executed" &&
      message.package_id === artifact.package_id,
  );
  assert.equal(
    scripts.length,
    1,
    "missing or ambiguous driver build-script identity",
  );
  const value = (key) => {
    const entries = scripts[0].env.filter((entry) => entry[0] === key);
    assert.equal(entries.length, 1, `missing or ambiguous ${key}`);
    return entries[0][1];
  };
  const compiledSourceSha = value("OFOX_BUILD_SOURCE_SHA");
  assert.equal(
    compiledSourceSha,
    expectedSha,
    "actual compiled source SHA differs",
  );
  assert.equal(
    value("OFOX_BUILD_SOURCE_CLEAN"),
    "true",
    "actual compiled source was dirty",
  );
  return {
    executable: artifact.executable,
    compiledSourceSha,
    compiledSourceClean: true,
  };
}

const instructions = (
  sha,
) => `# Windows parity Host driver — developer acceptance only

Source commit: ${sha}
This unsigned executable was compiled with Cargo's release profile. It does not
include an installer, checkout or controller dependencies. No paid task ran while
building this artifact. It uses the real AssistantHost with a temporary TestEnv
and a guarded loopback gateway; it does not certify the installed desktop UI.

Prerequisites: Git, pnpm 10.12.3 and an existing npm CLI for dependency preparation.
Rust/MSVC are not required on the acceptance client. Checkout this exact full SHA
and keep it clean. Extract this artifact and place the one authoritative campaign
directory outside the checkout (or in its ignored .real-eval directory).

In the exact checkout, run:

    pnpm install --frozen-lockfile
    pnpm assistant:install
    pnpm assistant:prepare
    pnpm assistant:smoke

These commands install locked JavaScript dependencies and prepare the pinned
bundled Node. They make no model requests. pnpm install alone does not install
the controller's sidecar dependencies. Compare this executable with SHA256SUMS.txt
and driver-provenance.json before use. Run the controller with
assistant-runtime/resources/node/node.exe, never with Cargo or a system Python.

See docs/document-engine-client-acceptance.md in the same checkout for exact live
and replay commands. Windows 11 live calls require the explicit --live and
--root-authorize-host win11 flags and the dedicated OFOX_API_KEY. Windows 10 uses
--replay-win10 with the six original Win11 captures. Transfer the same campaign
and budget ledger; never reset it or create another USD10 budget copy. Missing
keys, client evidence or manual reviews remain pending. No paid action is the
default. Only the controller may invoke the ignored ${entryPoint} test.
`;

export async function packageDriver({
  checkout,
  output,
  selected,
  expectedSha,
  toolchain,
}) {
  const deps = await realpath(
    path.join(checkout, "src-tauri/target/release/deps"),
  );
  const info = await lstat(selected.executable);
  assert.ok(
    info.isFile() && !info.isSymbolicLink(),
    "driver must be a regular file",
  );
  const executable = await realpath(selected.executable);
  assert.equal(
    path.dirname(executable),
    deps,
    "driver is outside the release test output",
  );
  assert.ok(path.isAbsolute(output), "artifact output must be absolute");
  // Resolve the existing destination parent so an outside junction cannot
  // create untracked package files inside the compiled checkout.
  output = path.join(
    await realpath(path.dirname(output)),
    path.basename(output),
  );
  const relative = path.relative(
    await realpath(checkout),
    path.resolve(output),
  );
  assert.ok(
    relative === ".." ||
      relative.startsWith(`..${path.sep}`) ||
      path.isAbsolute(relative),
    "artifact output must be outside the checkout",
  );
  const bytes = await readFile(executable);
  assertCleanCheckout(checkout, expectedSha);
  await mkdir(output, { mode: 0o700 }); // Existing packages are never overwritten.
  const provenance = {
    schemaVersion: 1,
    sourceSha: expectedSha,
    compiledSourceSha: selected.compiledSourceSha,
    compiledSourceClean: selected.compiledSourceClean,
    target: "x86_64-pc-windows-msvc",
    profile: "release",
    entryPoint,
    paidModelCalls: 0,
    installedUiAcceptance: "pending",
    rustc: toolchain.trim(),
    artifact: { file: binaryName, bytes: bytes.length, sha256: hash(bytes) },
  };
  await writeFile(path.join(output, binaryName), bytes, {
    flag: "wx",
    mode: 0o700,
  });
  await writeFile(
    path.join(output, "driver-provenance.json"),
    `${JSON.stringify(provenance, null, 2)}\n`,
    { flag: "wx" },
  );
  await writeFile(
    path.join(output, "SHA256SUMS.txt"),
    `${provenance.artifact.sha256}  ${binaryName}\n`,
    { flag: "wx" },
  );
  await writeFile(path.join(output, "README.md"), instructions(expectedSha), {
    flag: "wx",
  });
}

export async function main(argv = process.argv.slice(2)) {
  assert.equal(
    process.platform,
    "win32",
    "compile this artifact on native Windows",
  );
  assert.equal(process.arch, "x64");
  const options = {};
  for (let index = 0; index < argv.length; index += 2) {
    assert.ok(
      ["--sha", "--out"].includes(argv[index]) && argv[index + 1],
      "expected --sha SHA --out ABS_DIRECTORY",
    );
    assert.ok(!options[argv[index]], "duplicate option");
    options[argv[index]] = argv[index + 1];
  }
  const expectedSha = options["--sha"];
  assert.ok(
    typeof options["--out"] === "string" && path.isAbsolute(options["--out"]),
    "--out must be an absolute artifact directory",
  );
  assertCleanCheckout(root, expectedSha);
  const toolchain = execFileSync("rustc", ["-vV"], { encoding: "utf8" });
  assert.match(toolchain, /^host: x86_64-pc-windows-msvc$/m);
  const output = execFileSync(
    "cargo",
    [
      "test",
      "--locked",
      "--manifest-path",
      "src-tauri/Cargo.toml",
      "--release",
      "--test",
      "assistant_host",
      "--no-run",
      "--message-format=json",
    ],
    {
      cwd: root,
      encoding: "utf8",
      maxBuffer: 64 * 1024 * 1024,
      stdio: ["ignore", "pipe", "inherit"],
      env: sourceEnvironment(),
    },
  );
  const selected = selectCargoDriver(output, expectedSha);
  await packageDriver({
    checkout: root,
    output: options["--out"],
    selected,
    expectedSha,
    toolchain,
  });
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href)
  await main();
