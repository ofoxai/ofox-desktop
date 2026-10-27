// @vitest-environment node
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdtemp, readFile, writeFile, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import * as feeds from "../../scripts/update-manifests.mjs";

const bucket = "ofox-desktop-releases";
const endpoint = "https://fixture.r2.cloudflarestorage.com";
const base = "https://desktop.ofox.ai";
const macKey = (v: string) => `release/mac/arm/ofox_desktop_${v}.dmg`;
const winKey = (v: string) => `release/windows/x64/ofox_desktop_${v}.exe`;
const old = {
  version: "1.3.8",
  notes: "",
  pubDate: "2026-10-26T00:00:00Z",
  downloadPage: "https://ofox.ai/download",
  downloads: {
    "darwin-aarch64": base + "/" + macKey("1.3.8"),
    "windows-x86_64": base + "/" + winKey("1.3.8"),
  },
};
const roots: string[] = [];
afterEach(async () => {
  await Promise.all(
    roots.splice(0).map((p) => rm(p, { recursive: true, force: true })),
  );
});

async function fixture() {
  const root = await mkdtemp(join(tmpdir(), "ofox-publisher-"));
  roots.push(root);
  const objects = new Map<string, Buffer>();
  const calls: { op: string; key: string; success: boolean }[] = [];
  const etag = (b: Buffer) =>
    '"' + createHash("sha256").update(b).digest("hex").slice(0, 32) + '"';
  objects.set("latest.json", Buffer.from(JSON.stringify(old)));
  let hook: ((key: string, data: Buffer) => Promise<void>) | undefined;
  let readFailure: string | undefined;
  const fail = (code: string) =>
    Object.assign(new Error("fixture failure"), {
      stderr: `An error occurred (${code}) when calling the object operation`,
      code: 254,
    });
  const run = async (_tool: string, args: string[]) => {
    const op = args[1],
      key = args[args.indexOf("--key") + 1];
    const current = objects.get(key);
    if (op === "get-object") {
      if (readFailure === key) throw fail("AccessDenied");
      if (!current) throw fail("NoSuchKey");
      await writeFile(args[args.length - 1], current);
      calls.push({ op, key, success: true });
      return JSON.stringify({ ETag: etag(current) });
    }
    if (op !== "put-object") throw new Error("unexpected command " + op);
    const match = args.indexOf("--if-match"),
      absent = args.indexOf("--if-none-match");
    expect(match >= 0 || absent >= 0).toBe(true);
    if (
      (absent >= 0 && current) ||
      (match >= 0 && (!current || args[match + 1] !== etag(current)))
    ) {
      calls.push({ op, key, success: false });
      throw fail("PreconditionFailed");
    }
    const data = await readFile(args[args.indexOf("--body") + 1]);
    objects.set(key, data);
    calls.push({ op, key, success: true });
    await hook?.(key, data);
    return JSON.stringify({ ETag: etag(data) });
  };
  async function input(version = "1.3.9", both = true) {
    const mac = join(root, "input-" + version + ".dmg");
    await writeFile(mac, "mac-" + version);
    const windows = join(root, "input-" + version + ".exe");
    await writeFile(windows, "windows-" + version);
    return {
      bucket,
      endpoint,
      version,
      pubDate: "2026-10-27T00:00:00Z",
      minimumMacOS: "13.5",
      output: join(root, "feeds-" + version),
      artifacts: [
        { platform: "darwin-aarch64", path: mac },
        ...(both ? [{ platform: "windows-x86_64", path: windows }] : []),
      ],
      run,
      verifyPublic: async () => {},
    };
  }
  return {
    root,
    objects,
    calls,
    run,
    input,
    setHook: (h: typeof hook) => {
      hook = h;
    },
    setReadFailure: (key: string | undefined) => {
      readFailure = key;
    },
  };
}

describe("conditional update publishing", () => {
  it("serializes CI and emergency publishers through the same remote lease", async () => {
    const f = await fixture();
    const first = await f.input();
    const second = await f.input("1.3.10", false);
    let resume!: () => void;
    let acquired!: () => void;
    const paused = new Promise<void>((r) => {
      resume = r;
    });
    const ready = new Promise<void>((r) => {
      acquired = r;
    });
    f.setHook(async (key, data) => {
      if (
        key === feeds.UPDATE_FEED_LOCK_KEY &&
        JSON.parse(data.toString()).state === "active"
      ) {
        acquired();
        await paused;
      }
    });
    const publishing = feeds.publishUpdateRelease(first);
    await ready;
    await expect(feeds.publishUpdateRelease(second)).rejects.toThrow("Locked");
    resume();
    await publishing;
    expect(f.objects.get(macKey("1.3.10"))).toBeUndefined();
    expect(
      JSON.parse(f.objects.get("latest.json")!.toString()).downloads[
        "windows-x86_64"
      ],
    ).toContain("_1.3.9.exe");
  });
  it("repairs a modern-first partial publication while retrying only the Mac platform", async () => {
    const f = await fixture();
    const prior = feeds.buildUpdateManifests({
      legacy: old,
      modern: null,
      version: "1.3.9",
      platforms: ["darwin-aarch64", "windows-x86_64"],
      pubDate: "2026-10-27T00:00:00Z",
      minimumMacOS: "13.5",
    });
    f.objects.set("latest-v2.json", Buffer.from(JSON.stringify(prior.modern)));
    await feeds.publishUpdateRelease(await f.input("1.3.9", false));
    const legacy = JSON.parse(f.objects.get("latest.json")!.toString());
    expect(legacy.downloads["windows-x86_64"]).toContain("_1.3.9.exe");
    expect(legacy.downloads["darwin-aarch64"]).toContain("_1.3.8.dmg");
  });
  it("allows identical-byte retries and refuses replacement of a published installer", async () => {
    const f = await fixture();
    const input = await f.input();
    await feeds.publishUpdateRelease(input);
    await feeds.publishUpdateRelease(input);
    expect(
      f.calls.filter(
        (c) => c.op === "put-object" && c.key === macKey("1.3.9") && c.success,
      ),
    ).toHaveLength(1);
    await writeFile(input.artifacts[0].path, "different source bytes");
    await expect(feeds.publishUpdateRelease(input)).rejects.toThrow(
      "Immutable",
    );
    expect(f.objects.get(macKey("1.3.9"))!.toString()).toBe("mac-1.3.9");
  });
  it("refuses unreadable manifests before uploading any installer", async () => {
    const f = await fixture();
    f.setReadFailure("latest.json");
    await expect(feeds.publishUpdateRelease(await f.input())).rejects.toThrow();
    expect(
      f.calls.filter((c) => c.op === "put-object" && c.key.endsWith(".dmg")),
    ).toHaveLength(0);
  });
  it.each(["null", "[]", "not json"])(
    "does not bootstrap over a damaged modern object: %s",
    async (content) => {
      const f = await fixture();
      f.objects.set("latest-v2.json", Buffer.from(content));
      await expect(
        feeds.publishUpdateRelease(await f.input()),
      ).rejects.toThrow();
      expect(
        f.calls.filter((c) => c.op === "put-object" && c.key.endsWith(".dmg")),
      ).toHaveLength(0);
    },
  );
  it("dry-run performs no remote writes or lock acquisition", async () => {
    const f = await fixture();
    await feeds.publishUpdateRelease({ ...(await f.input()), dryRun: true });
    expect(f.calls.filter((c) => c.op === "put-object")).toHaveLength(0);
  });
  it("does not expire or steal another publisher's active lock", async () => {
    const f = await fixture();
    f.objects.set(
      feeds.UPDATE_FEED_LOCK_KEY,
      Buffer.from(
        JSON.stringify({
          schemaVersion: 1,
          state: "active",
          owner: "another",
          createdAt: "2000-01-01",
        }),
      ),
    );
    await expect(feeds.publishUpdateRelease(await f.input())).rejects.toThrow(
      "Locked",
    );
    expect(f.calls.filter((c) => c.op === "put-object")).toHaveLength(0);
  });
  it("keeps an uncertain lock acquisition for operator recovery without replay", async () => {
    const f = await fixture();
    f.setHook(async (key, data) => {
      if (
        key === feeds.UPDATE_FEED_LOCK_KEY &&
        JSON.parse(data.toString()).state === "active"
      )
        throw Object.assign(new Error("timeout"), {
          stderr: "network timeout",
        });
    });
    await expect(feeds.publishUpdateRelease(await f.input())).rejects.toThrow();
    expect(
      JSON.parse(f.objects.get(feeds.UPDATE_FEED_LOCK_KEY)!.toString()).state,
    ).toBe("active");
    expect(f.calls.filter((c) => c.op === "put-object")).toHaveLength(1);
  });
  it("uses feed ETags to reject interference from an older publisher", async () => {
    const f = await fixture();
    f.setHook(async (key) => {
      if (key === "latest-v2.json")
        f.objects.set(
          "latest.json",
          Buffer.from(JSON.stringify({ ...old, notes: "concurrent change" })),
        );
    });
    await expect(feeds.publishUpdateRelease(await f.input())).rejects.toThrow();
    expect(JSON.parse(f.objects.get("latest.json")!.toString()).notes).toBe(
      "concurrent change",
    );
  });
  it("does not automatically repeat an uncertain installer upload", async () => {
    const f = await fixture();
    const input = await f.input();
    f.setHook(async (key) => {
      if (key === macKey("1.3.9"))
        throw Object.assign(new Error("timeout"), {
          stderr: "network timeout",
        });
    });
    await expect(feeds.publishUpdateRelease(input)).rejects.toThrow();
    expect(
      f.calls.filter((c) => c.op === "put-object" && c.key === macKey("1.3.9")),
    ).toHaveLength(1);
    expect(f.objects.get("latest-v2.json")).toBeUndefined();
    f.setHook(undefined);
    await feeds.publishUpdateRelease(input);
    expect(f.objects.get(macKey("1.3.9"))!.toString()).toBe("mac-1.3.9");
  });
  it("holds the lease through public verification and releases it after failure", async () => {
    const f = await fixture();
    const input = await f.input();
    input.verifyPublic = async () => {
      expect(
        JSON.parse(f.objects.get(feeds.UPDATE_FEED_LOCK_KEY)!.toString()).state,
      ).toBe("active");
      throw new Error("fixture public mismatch");
    };
    await expect(feeds.publishUpdateRelease(input)).rejects.toThrow(
      "fixture public mismatch",
    );
    expect(
      JSON.parse(f.objects.get(feeds.UPDATE_FEED_LOCK_KEY)!.toString()).state,
    ).toBe("idle");
  });
});

describe("emergency shell invocation", () => {
  it.skipIf(process.platform !== "darwin")(
    "passes normal and dry-run arguments on the system Bash 3.2",
    async () => {
      const source = await readFile("scripts/release.sh", "utf8");
      const fragment = source
        .split("trap 'rm -rf \"$FEED_DIR\"' EXIT\n")[1]
        ?.split('if [[ "$DRY_RUN" -eq 1 ]]; then')[0];
      expect(fragment).toBeDefined();
      for (const dry of ["0", "1"]) {
        const output = execFileSync(
          "/bin/bash",
          ["-uc", 'node() { printf "%s\\0" "$@"; }\n' + fragment],
          {
            encoding: "utf8",
            env: {
              PATH: process.env.PATH,
              DRY_RUN: dry,
              BUCKET: "fixture",
              ENDPOINT: endpoint,
              VERSION: "1.3.9",
              PLATFORM_KEY: "darwin-aarch64",
              DMG_SRC: "/tmp/fixture installer.dmg",
              NOTES: "fixture notes",
              FEED_DIR: "/tmp/fixture feed",
            },
          },
        );
        const args = output.split("\0").filter(Boolean);
        expect(args).toContain("darwin-aarch64=/tmp/fixture installer.dmg");
        expect(args.includes("--dry-run")).toBe(dry === "1");
      }
    },
  );
});
