// @vitest-environment node
import { mkdtemp, mkdir, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import {
  recordBuild,
  verifyRelease,
  checkRelease,
} from "../../scripts/release-artifacts.mjs";

const sha = "a".repeat(40);
const version = "1.3.6";
const roots: string[] = [];
async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "ofox-release-proof-"));
  roots.push(root);
  const artifactsRoot = join(root, "artifacts");
  const provenanceRoot = join(root, "provenance");
  const notaryRoot = join(root, "notary");
  await mkdir(join(notaryRoot, "notary-darwin-aarch64"), { recursive: true });
  await writeFile(
    join(notaryRoot, "notary-darwin-aarch64", "submission-id"),
    "11111111-2222-4333-8444-555555555555",
  );
  for (const [platform, ext] of [
    ["darwin-aarch64", "dmg"],
    ["windows-x86_64", "exe"],
  ]) {
    const dir = join(artifactsRoot, `build-${platform}`);
    const proof = join(provenanceRoot, `provenance-${platform}`);
    await mkdir(dir, { recursive: true });
    await mkdir(proof, { recursive: true });
    await writeFile(
      join(dir, `ofox_desktop_${version}.${ext}`),
      `original-${platform}`,
    );
    await recordBuild({
      root: dir,
      output: join(proof, "build-provenance.json"),
      platform,
      version,
      sourceSha: sha,
    });
  }
  const calls: string[] = [];
  const run = async (command: string, args: string[]) => {
    calls.push(`${command} ${args.slice(0, 3).join(" ")}`);
    if (args[0] === "notarytool" && args[1] === "info")
      return JSON.stringify({ status: "Accepted" });
    if (args[0] === "stapler" && args[1] === "staple")
      await writeFile(args[2], "stapled-final-bytes");
    if (command === "hdiutil" && args[0] === "attach") {
      const mount = args[args.indexOf("-mountpoint") + 1];
      await mkdir(join(mount, "Ofox Desktop.app"), { recursive: true });
    }
    if (command === "codesign" && args[0] === "-dv")
      return "Authority=Developer ID Application: Fixture\nTeamIdentifier=TEAM123\nCDHash=0123456789";
    if (command === "plutil")
      return args[1] === "CFBundleIdentifier" ? "ai.ofox.desktop" : version;
    return "";
  };
  return {
    artifactsRoot,
    provenanceRoot,
    notaryRoot,
    sourceSha: sha,
    version,
    expectedTeamId: "TEAM123",
    run,
    calls,
    auth: { appleId: "fixture", password: "fixture", teamId: "TEAM123" },
  };
}
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((root) => rm(root, { recursive: true, force: true })),
  );
});

describe("verified release artifacts", () => {
  it("hashes the stapled bytes and checks App identity before marking a source SHA verified", async () => {
    const input = await fixture();
    await verifyRelease(input);
    const proof = await checkRelease(input);
    expect(proof.sourceSha).toBe(sha);
    expect(proof.platforms[0].notarization?.status).toBe("Accepted");
    expect(
      input.calls.some((call) =>
        call.startsWith("codesign --verify --deep --strict"),
      ),
    ).toBe(true);
    expect(
      input.calls.some((call) =>
        call.startsWith("spctl --assess --type execute"),
      ),
    ).toBe(true);
    expect(
      await readFile(
        join(
          input.artifactsRoot,
          "build-darwin-aarch64",
          `ofox_desktop_${version}.dmg`,
        ),
        "utf8",
      ),
    ).toBe("stapled-final-bytes");
    await writeFile(
      join(
        input.artifactsRoot,
        "build-darwin-aarch64",
        `ofox_desktop_${version}.dmg`,
      ),
      "original-darwin-aarch64",
    );
    await expect(checkRelease(input)).rejects.toThrow("artifactChanged");
  });
  it("refuses another source SHA, changed build output and rejected notarization", async () => {
    const input = await fixture();
    await expect(
      verifyRelease({ ...input, sourceSha: "b".repeat(40) }),
    ).rejects.toThrow("buildProvenanceMismatch");
    await writeFile(
      join(
        input.artifactsRoot,
        "build-windows-x86_64",
        `ofox_desktop_${version}.exe`,
      ),
      "changed",
    );
    await expect(verifyRelease(input)).rejects.toThrow("artifactChanged");
    expect(input.calls).toEqual([]);
    const other = await fixture();
    await expect(
      verifyRelease({
        ...other,
        run: async () => JSON.stringify({ status: "Invalid" }),
      }),
    ).rejects.toThrow("notarizationRejected");
  });
  it("requires Developer ID team identity and refuses missing platforms or extra files", async () => {
    const input = await fixture();
    await expect(
      verifyRelease({ ...input, expectedTeamId: "OTHER" }),
    ).rejects.toThrow("signingIdentityMismatch");
    const fresh = await fixture();
    await writeFile(
      join(fresh.artifactsRoot, "build-windows-x86_64", "unexpected.txt"),
      "no",
    );
    await expect(verifyRelease(fresh)).rejects.toThrow("unexpectedArtifact");
    const missing = await fixture();
    await rm(join(missing.artifactsRoot, "build-windows-x86_64"), {
      recursive: true,
    });
    await expect(verifyRelease(missing)).rejects.toThrow("unexpectedArtifact");
  });
  it("rejects installers changed while notarization is being verified", async () => {
    const input = await fixture();
    const original = input.run;
    await expect(
      verifyRelease({
        ...input,
        run: async (command, args) => {
          if (command === "hdiutil" && args[0] === "attach")
            await writeFile(
              join(
                input.artifactsRoot,
                "build-windows-x86_64",
                `ofox_desktop_${version}.exe`,
              ),
              "late mutation",
            );
          return original(command, args);
        },
      }),
    ).rejects.toThrow("artifactChanged");
  });
});
