import { createHash } from "node:crypto";
import { execFile } from "node:child_process";
import { promisify } from "node:util";
import { lstat, readFile, realpath } from "node:fs/promises";
import { tmpdir } from "node:os";
import { basename, dirname, isAbsolute, join, relative, sep } from "node:path";
import { profilePaths } from "./native-acceptance-drivers.mjs";

const exec = promisify(execFile);
const hash = (value) => createHash("sha256").update(value).digest("hex");
const inside = (root, path) => {
  const rel = relative(root, path);
  return (
    rel !== "" &&
    !isAbsolute(rel) &&
    rel !== ".." &&
    !rel.startsWith(`..${sep}`)
  );
};
async function bounded(path, max = 4 * 1024 * 1024) {
  const stat = await lstat(path);
  if (!stat.isFile() || stat.isSymbolicLink() || stat.size > max)
    throw new Error("evidenceFileInvalid");
  return await readFile(path);
}
export async function verifyAcceptanceIdentity(
  manifest,
  { checkoutRoot } = {},
) {
  const identity = {
    status: "unverified",
    requestedSourceSha: manifest.sourceSha,
    sourceSha: null,
    checkout: { status: "notProvided" },
    artifact: { status: "notProvided" },
  };
  if (checkoutRoot) {
    try {
      const { stdout: head } = await exec(
        "git",
        ["-C", checkoutRoot, "rev-parse", "HEAD"],
        { encoding: "utf8", timeout: 5000, maxBuffer: 1024 * 1024 },
      );
      const { stdout: status } = await exec(
        "git",
        [
          "-C",
          checkoutRoot,
          "status",
          "--porcelain",
          "--untracked-files=normal",
        ],
        { encoding: "utf8", timeout: 5000, maxBuffer: 1024 * 1024 },
      );
      const changes = status
        .trim()
        .split(/\r?\n/)
        .filter((line) => line && line !== "?? HANDOFF-codex.md");
      identity.checkout = {
        status:
          head.trim() !== manifest.sourceSha
            ? "shaMismatch"
            : changes.length
              ? "uncommittedChanges"
              : "verified",
        commitSha: head.trim(),
        clean: changes.length === 0,
      };
    } catch {
      identity.checkout = { status: "unavailable" };
    }
  }
  if (manifest.artifact) {
    const spec = manifest.artifact;
    try {
      if (
        !isAbsolute(spec.path ?? "") ||
        !isAbsolute(spec.provenancePath ?? "") ||
        !/^[a-f0-9]{64}$/.test(spec.sha256 ?? "") ||
        !/^[a-f0-9]{64}$/.test(spec.provenanceSha256 ?? "")
      )
        throw new Error("artifactIdentityInvalid");
      const artifact = await bounded(spec.path, 2 * 1024 * 1024 * 1024);
      const provenanceBytes = await bounded(spec.provenancePath);
      const provenance = JSON.parse(provenanceBytes.toString("utf8"));
      if (
        hash(artifact) !== spec.sha256 ||
        hash(provenanceBytes) !== spec.provenanceSha256 ||
        (provenance.sourceCommit ?? provenance.sourceSha) !== manifest.sourceSha
      )
        throw new Error("artifactIdentityMismatch");
      identity.artifact = {
        status: "verified",
        sha256: spec.sha256,
        bytes: artifact.length,
        provenanceSha256: spec.provenanceSha256,
        sourceSha: manifest.sourceSha,
        evidence: "explicitOperatorBoundArtifact",
      };
    } catch {
      identity.artifact = { status: "mismatchOrUnavailable" };
    }
  }
  if (
    identity.checkout.status === "verified" &&
    identity.artifact.status === "verified"
  ) {
    identity.status = "verified";
    identity.sourceSha = manifest.sourceSha;
  }
  return identity;
}

export async function profileResourceEvidence(profile) {
  const result = {};
  for (const [kind, path] of Object.entries({
    instructions: profile.instructionPath,
    mcp: profile.mcpNativePath ?? profile.mcpConfig,
    skills: profile.skillPath,
  })) {
    const bytes = await bounded(path);
    result[kind] = {
      sha256: hash(bytes),
      bytes: bytes.length,
      version: "contentSha256",
    };
  }
  return result;
}

// A receipt is supplied by the caller's existing OFox sandbox writer, not by
// model text. The caller pins its exact digest; file checks do not grant native
// ownership, infer a successful AI call, or authorize an arbitrary path.
export async function verifyWriterProfile(
  spec,
  { sourceSha, agentId, now = Date.now() },
) {
  if (
    !spec ||
    !isAbsolute(spec.root ?? "") ||
    !isAbsolute(spec.receiptPath ?? "") ||
    !/^[a-f0-9]{64}$/.test(spec.receiptSha256 ?? "")
  )
    throw new Error("writerProfileInvalid");
  const root = await realpath(spec.root);
  if (!inside(await realpath(tmpdir()), root))
    throw new Error("writerProfileOutsideSandbox");
  const receiptPath = await realpath(spec.receiptPath);
  if (!inside(root, receiptPath))
    throw new Error("writerReceiptOutsideSandbox");
  const bytes = await bounded(receiptPath);
  if (hash(bytes) !== spec.receiptSha256)
    throw new Error("writerReceiptChanged");
  const receipt = JSON.parse(bytes.toString("utf8"));
  const marker = JSON.parse(
    (await bounded(join(root, ".ofox-acceptance-prepared.json"))).toString(
      "utf8",
    ),
  );
  if (
    receipt.schemaVersion !== 1 ||
    receipt.writer !== "ofoxSandboxWriterV1" ||
    receipt.sourceSha !== sourceSha ||
    receipt.compiledSourceSha !== sourceSha ||
    receipt.compiledSourceClean !== true ||
    receipt.agentId !== agentId ||
    (await realpath(receipt.root ?? "")) !== root ||
    marker.schemaVersion !== 1 ||
    marker.sourceSha !== sourceSha ||
    marker.agentId !== agentId ||
    marker.preparedRunId !== receipt.preparedRunId ||
    !Number.isSafeInteger(marker.expiresAt) ||
    marker.expiresAt <= now
  )
    throw new Error("writerReceiptIdentityMismatch");
  if (
    !Array.isArray(receipt.operations) ||
    !["instructions", "mcp", "skills"].every((kind) =>
      receipt.operations.some(
        (operation) =>
          operation.kind === kind &&
          operation.result === "applied" &&
          typeof operation.operationId === "string" &&
          operation.operationId.length > 0 &&
          /^[a-f0-9]{64}$/.test(operation.receiptSha256 ?? ""),
      ),
    )
  )
    throw new Error("writerOperationsIncomplete");
  if (
    !Array.isArray(receipt.files) ||
    receipt.files.length > 1000 ||
    !["instructions", "mcp", "skills", "proof"].every((kind) =>
      receipt.files.some((file) => file.kind === kind),
    )
  )
    throw new Error("writerFilesIncomplete");
  const seen = new Set();
  for (const file of receipt.files) {
    if (
      typeof file.relativePath !== "string" ||
      isAbsolute(file.relativePath) ||
      file.relativePath.split(/[\\/]/).includes("..") ||
      !inside(root, join(root, file.relativePath)) ||
      seen.has(file.relativePath) ||
      !/^[a-f0-9]{64}$/.test(file.sha256 ?? "")
    )
      throw new Error("writerFilePathInvalid");
    seen.add(file.relativePath);
    const target = await realpath(join(root, file.relativePath));
    if (!inside(root, target) || hash(await bounded(target)) !== file.sha256)
      throw new Error("writerFileChanged");
  }
  const paths = profilePaths(root, agentId);
  for (const [kind, path] of Object.entries({
    instructions: paths.instructionPath,
    mcp: paths.mcpNativePath,
    skills: paths.skillPath,
  }))
    if (
      !receipt.files.some(
        (file) =>
          file.kind === kind &&
          file.relativePath.replaceAll("\\", "/") ===
            relative(root, path).replaceAll("\\", "/"),
      )
    )
      throw new Error("writerFilesIncomplete");
  if (
    agentId === "claude" &&
    !receipt.files.some(
      (file) => file.kind === "mcp" && file.relativePath === "mcp.json",
    )
  )
    throw new Error("writerFilesIncomplete");
  const proofBytes = await bounded(join(root, "fixture-private.json"));
  if (
    hash(proofBytes) !== marker.privateProofSha256 ||
    !receipt.files.some(
      (file) =>
        file.relativePath === "fixture-private.json" &&
        file.sha256 === marker.privateProofSha256 &&
        file.kind === "proof",
    )
  )
    throw new Error("writerPrivateProofMismatch");
  const proof = JSON.parse(proofBytes.toString("utf8"));
  if (
    proof.runId !== receipt.preparedRunId ||
    proof.sourceSha !== sourceSha ||
    proof.agentId !== agentId ||
    typeof proof.receipt !== "string" ||
    basename(proof.receipt) !== "mcp-receipt.json" ||
    (await realpath(dirname(proof.receipt))) !== root
  )
    throw new Error("writerPrivateProofMismatch");
  return {
    root,
    receipt,
    proof,
    receiptSha256: spec.receiptSha256,
    writer: "ofoxSandboxWriterV1",
    verifiedFileCount: seen.size,
  };
}
