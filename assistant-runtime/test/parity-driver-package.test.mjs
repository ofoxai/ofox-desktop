import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import {
  mkdir,
  mkdtemp,
  readFile,
  rm,
  symlink,
  writeFile,
} from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import {
  assertCleanCheckout,
  packageDriver,
  runGatewayPreflight,
  selectCargoDriver,
  sourceEnvironment,
} from "../../scripts/package-parity-driver.mjs";

const sourceSha = "a".repeat(40);
const packageId =
  "path+file:///fixture/ofox-desktop/src-tauri#ofox-switch@1.5.2";
const cargoMessages = () => [
  {
    reason: "build-script-executed",
    package_id: packageId,
    env: [
      ["OFOX_BUILD_SOURCE_SHA", sourceSha],
      ["OFOX_BUILD_SOURCE_CLEAN", "true"],
    ],
  },
  {
    reason: "compiler-artifact",
    target: { name: "assistant_host", kind: ["lib"] },
    executable: null,
  },
  {
    reason: "compiler-artifact",
    package_id: packageId,
    target: { name: "assistant_host", kind: ["test"] },
    profile: { test: true, opt_level: "s", debug_assertions: false },
    executable:
      "C:\\fixture\\src-tauri\\target\\release\\deps\\assistant_host-0123456789abcdef.exe",
    fresh: true,
  },
  { reason: "build-finished", success: true },
];
const cargoJson = (messages) =>
  messages.map((message) => JSON.stringify(message)).join("\r\n");

test("Windows case-insensitive Git environment aliases cannot redirect build provenance", () => {
  const environment = sourceEnvironment({
    PATH: "fixture-toolchain",
    git_dir: "foreign-index",
    git_work_tree: "foreign-worktree",
    GiT_InDeX_FiLe: "foreign-index-file",
    GIT_CONFIG_GLOBAL: "foreign-config",
  });
  assert.equal(environment.PATH, "fixture-toolchain");
  assert.equal(Object.hasOwn(environment, "git_dir"), false);
  assert.equal(Object.hasOwn(environment, "git_work_tree"), false);
  assert.equal(Object.hasOwn(environment, "GiT_InDeX_FiLe"), false);
  assert.equal(environment.GIT_CONFIG_GLOBAL, "/dev/null");
  assert.equal(environment.GIT_CONFIG_NOSYSTEM, "1");
});

test("Cargo evidence selects the integration test and its actual compiled identity even on a warm build", () => {
  const selected = selectCargoDriver(cargoJson(cargoMessages()), sourceSha);
  assert.equal(selected.executable, cargoMessages()[2].executable);
  assert.equal(selected.compiledSourceSha, sourceSha);
  assert.equal(selected.compiledSourceClean, true);
});

test("Cargo evidence rejects ambiguous executables, failed builds and forged compiled identity", () => {
  for (const change of [
    (rows) => rows.splice(2, 0, structuredClone(rows[2])),
    (rows) => {
      rows.at(-1).success = false;
    },
    (rows) => {
      rows[0].env[0][1] = "b".repeat(40);
    },
    (rows) => {
      rows[0].env[1][1] = "false";
    },
    (rows) => {
      rows[0].package_id = "unrelated-package";
    },
    (rows) => {
      rows[2].profile.test = false;
    },
    (rows) => {
      rows[2].profile.opt_level = "0";
    },
    (rows) => {
      rows[0].env.push(["OFOX_BUILD_SOURCE_SHA", sourceSha]);
    },
  ]) {
    const rows = cargoMessages();
    change(rows);
    assert.throws(() => selectCargoDriver(cargoJson(rows), sourceSha));
  }
});

test("release gateway preflight uses exactly its keyless test and a private home", async () => {
  let calls = 0;
  const receipt = await runGatewayPreflight({
    executable: "synthetic-driver-never-executed.exe",
    checkout: process.cwd(),
    environment: {
      PATH: "synthetic-path",
      SystemRoot: "synthetic-system-root",
      OFOX_API_KEY: "synthetic-must-not-be-forwarded",
      OFOX_REAL_EVAL_KEY: "synthetic-must-not-be-forwarded",
      OFOX_PARITY_PROXY_TOKEN: "synthetic-must-not-be-forwarded",
      HTTPS_PROXY: "synthetic-must-not-be-forwarded",
      GITHUB_TOKEN: "synthetic-must-not-be-forwarded",
    },
    execute: (executable, args, options) => {
      calls++;
      assert.equal(executable, "synthetic-driver-never-executed.exe");
      assert.deepEqual(args, [
        "parity_eval::release_gateway_preflight",
        "--ignored",
        "--exact",
        "--nocapture",
        "--test-threads=1",
      ]);
      assert.equal(options.timeout, 120_000);
      assert.equal(options.env.PATH, "synthetic-path");
      for (const name of [
        "OFOX_API_KEY",
        "OFOX_REAL_EVAL_KEY",
        "OFOX_PARITY_PROXY_TOKEN",
        "HTTPS_PROXY",
        "GITHUB_TOKEN",
      ])
        assert.equal(Object.hasOwn(options.env, name), false);
      for (const name of [
        "USERPROFILE",
        "TEMP",
        "CC_SWITCH_TEST_HOME",
        "APPDATA",
      ])
        assert.equal(options.env[name], options.env.HOME);
      return "test result: ok. 1 passed; 0 failed; 0 ignored; 100 filtered out\n";
    },
  });
  assert.equal(calls, 1);
  assert.deepEqual(receipt, {
    entryPoint: "parity_eval::release_gateway_preflight",
    status: "passed",
    paidModelCalls: 0,
  });
  for (const output of [
    "test result: ok. 0 passed; 0 failed;",
    "test result: FAILED. 0 passed; 1 failed;",
  ])
    await assert.rejects(
      runGatewayPreflight({
        executable: "not-executed.exe",
        checkout: process.cwd(),
        execute: () => output,
      }),
      /did not run exactly one/,
    );
});

async function checkoutFixture(t) {
  const directory = await mkdtemp(
    path.join(os.tmpdir(), "ofox-parity-driver-"),
  );
  t.after(() => rm(directory, { recursive: true, force: true }));
  const checkout = path.join(directory, "checkout");
  await mkdir(checkout);
  await writeFile(path.join(checkout, "tracked.txt"), "original\n");
  await writeFile(path.join(checkout, ".gitignore"), "/src-tauri/target/\n");
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: checkout,
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
      env: sourceEnvironment(),
    });
  git("init", "--quiet");
  git("add", ".");
  git(
    "-c",
    "user.name=Fixture",
    "-c",
    "user.email=fixture@example.invalid",
    "-c",
    "commit.gpgsign=false",
    "commit",
    "--quiet",
    "-m",
    "fixture",
  );
  return { directory, checkout, sha: git("rev-parse", "HEAD").trim() };
}

test("real Git state rejects both tracked drift and new untracked package outputs", async (t) => {
  const { checkout, sha } = await checkoutFixture(t);
  assertCleanCheckout(checkout, sha);
  assert.throws(
    () => assertCleanCheckout(checkout, "b".repeat(40)),
    /HEAD differs/,
  );
  await writeFile(path.join(checkout, "tracked.txt"), "changed\n");
  assert.throws(() => assertCleanCheckout(checkout, sha), /clean checkout/);
  await writeFile(path.join(checkout, "tracked.txt"), "original\n");
  await mkdir(path.join(checkout, "dist-release"));
  await writeFile(
    path.join(checkout, "dist-release", "new-package.exe"),
    "fixture",
  );
  assert.throws(() => assertCleanCheckout(checkout, sha), /untracked/);
  const foreignTree = path.join(path.dirname(checkout), "foreign-worktree");
  await mkdir(foreignTree);
  await writeFile(path.join(foreignTree, "tracked.txt"), "original\n");
  await writeFile(path.join(foreignTree, ".gitignore"), "/src-tauri/target/\n");
  assert.throws(() =>
    execFileSync(
      process.execPath,
      [
        "--input-type=module",
        "-e",
        "const {assertCleanCheckout}=await import(process.argv[1]);assertCleanCheckout(process.argv[2],process.argv[3]);",
        new URL("../../scripts/package-parity-driver.mjs", import.meta.url)
          .href,
        checkout,
        sha,
      ],
      {
        stdio: "pipe",
        env: {
          ...process.env,
          GIT_DIR: path.join(checkout, ".git"),
          GIT_WORK_TREE: foreignTree,
        },
      },
    ),
  );
});

test("the artifact binds the selected executable bytes and never overwrites another package", async (t) => {
  const { directory, checkout, sha } = await checkoutFixture(t);
  const deps = path.join(checkout, "src-tauri/target/release/deps");
  await mkdir(deps, { recursive: true });
  const executable = path.join(deps, "assistant_host-0123456789abcdef.exe");
  const bytes = Buffer.from("synthetic driver bytes; never executed");
  await writeFile(executable, bytes);
  const output = path.join(directory, "artifact");
  const parameters = {
    checkout,
    output,
    selected: { executable, compiledSourceSha: sha, compiledSourceClean: true },
    expectedSha: sha,
    toolchain: "synthetic rustc fixture",
    gatewayPreflight: {
      entryPoint: "parity_eval::release_gateway_preflight",
      status: "passed",
      paidModelCalls: 0,
    },
  };
  await packageDriver(parameters);
  const receipt = JSON.parse(
    await readFile(path.join(output, "driver-provenance.json")),
  );
  assert.equal(receipt.sourceSha, sha);
  assert.equal(receipt.compiledSourceSha, sha);
  assert.equal(
    receipt.artifact.sha256,
    createHash("sha256").update(bytes).digest("hex"),
  );
  assert.deepEqual(
    await readFile(path.join(output, receipt.artifact.file)),
    bytes,
  );
  assert.equal(receipt.paidModelCalls, 0);
  assert.equal(receipt.gatewayPreflight.status, "passed");
  await assert.rejects(
    packageDriver({ ...parameters, gatewayPreflight: undefined }),
    /preflight/,
  );
  await assert.rejects(packageDriver(parameters), /EEXIST/);
  assert.deepEqual(
    await readFile(path.join(output, receipt.artifact.file)),
    bytes,
  );
  const alias = path.join(directory, "checkout-alias");
  await symlink(checkout, alias, "junction");
  await assert.rejects(
    packageDriver({ ...parameters, output: path.join(alias, "new-artifact") }),
    /outside the checkout/,
  );
});

test("the Windows driver compiles before package files can dirty source provenance", async () => {
  const workflow = await readFile(
    new URL("../../.github/workflows/branch-packages.yml", import.meta.url),
    "utf8",
  );
  const driver = workflow.indexOf("scripts/package-parity-driver.mjs");
  const installer = workflow.indexOf("- name: Prepare exact installer");
  assert.ok(driver >= 0 && driver < installer);
  assert.match(workflow, /assistant-parity-driver-windows-x86_64-/);
});
