import { readFile, writeFile, mkdir, mkdtemp, rm } from "node:fs/promises";
import path from "node:path";
import { tmpdir } from "node:os";
import { execFileSync } from "node:child_process";
import { fileURLToPath, pathToFileURL } from "node:url";

const repoRoot = path.resolve(
  path.dirname(fileURLToPath(import.meta.url)),
  "..",
);
const sourceLabel = "magpie market_skills.json (MIT, yetone)";

// Not recommended: paid media-generation routers that compete with Ofox's own
// generation, a repository reusing obra/superpowers' name, spam automation,
// and skills their authors mark retired or deprecated.
const excludedSources = new Set([
  "genmedia-labs/skills",
  "prime-skills/runcomfy-agent-skills",
  "101-skills/superpowers",
]);
const excludedSkills = new Set([
  "flowkit-labs/skills/reddit-automation",
  "leonxlnx/taste-skill/redesign-existing-projects",
  "prisma/skills/prisma-database-setup",
]);

export function selectPopularSkills(entries) {
  const counts = new Map();
  const seen = new Set();
  const selected = [];
  for (const entry of [...entries].sort((a, b) => b.installs - a.installs)) {
    if (
      !/^[\w-]+\/[\w.-]+$/.test(entry.source) ||
      !/^[\w.-]+$/.test(entry.skillId) ||
      typeof entry.name !== "string" ||
      !entry.name.trim() ||
      !Number.isFinite(entry.installs) ||
      entry.installs < 0
    )
      throw new Error("Invalid skill metadata");
    const source = entry.source.toLowerCase();
    const key = `${source}/${entry.skillId}`;
    if (excludedSources.has(source) || excludedSkills.has(key.toLowerCase()))
      continue;
    if ((counts.get(source) ?? 0) >= 4 || seen.has(key)) continue;
    counts.set(source, (counts.get(source) ?? 0) + 1);
    seen.add(key);
    selected.push({
      source: entry.source,
      skillId: entry.skillId,
      name: entry.name,
      installs: entry.installs,
      official: entry.isOfficial === true,
    });
    if (selected.length === 60) break;
  }
  return selected;
}

export function metaDescription(html) {
  for (const meta of html.match(/<meta\b[^>]*>/gi) ?? []) {
    const attributes = Object.fromEntries(
      [...meta.matchAll(/([\w-]+)\s*=\s*(["'])(.*?)\2/gs)].map((match) => [
        match[1].toLowerCase(),
        match[3],
      ]),
    );
    if (attributes.name?.toLowerCase() !== "description") continue;
    return (
      attributes.content
        ?.replace(
          /&(#x[\da-f]+|#\d+|amp|quot|apos|lt|gt|nbsp);/gi,
          (entity, value) => {
            if (value.startsWith("#")) {
              const code =
                value[1].toLowerCase() === "x"
                  ? parseInt(value.slice(2), 16)
                  : Number(value.slice(1));
              return code <= 0x10ffff ? String.fromCodePoint(code) : entity;
            }
            return (
              { amp: "&", quot: '"', apos: "'", lt: "<", gt: ">", nbsp: " " }[
                value.toLowerCase()
              ] ?? entity
            );
          },
        )
        .trim() || undefined
    );
  }
}

async function concurrent(items, action) {
  let index = 0;
  await Promise.all(
    Array.from({ length: Math.min(6, items.length) }, async () => {
      while (index < items.length) await action(items[index++]);
    }),
  );
}
async function request(url) {
  const response = await fetch(url, {
    signal: AbortSignal.timeout(20000),
    headers: { "User-Agent": "Ofox-Desktop-catalog-builder" },
  });
  if (!response.ok) throw new Error(`HTTP ${response.status}`);
  return response;
}

const pngSignature = Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]);
async function avatarPng(bytes) {
  if (bytes.subarray(0, 8).equals(pngSignature)) return bytes;
  // GitHub may serve a JPEG at a .png URL. Conversion is generation-time only.
  // macOS uses its bundled sips; other platforms require ImageMagick's magick.
  const temporary = await mkdtemp(path.join(tmpdir(), "popular-avatar-"));
  try {
    const input = path.join(temporary, "avatar");
    const output = path.join(temporary, "avatar.png");
    await writeFile(input, bytes);
    if (process.platform === "darwin")
      execFileSync("sips", ["-s", "format", "png", input, "--out", output], {
        stdio: "pipe",
      });
    else execFileSync("magick", [input, "png:" + output], { stdio: "pipe" });
    const png = await readFile(output);
    if (!png.subarray(0, 8).equals(pngSignature))
      throw new Error("Avatar conversion did not produce PNG");
    return png;
  } finally {
    await rm(temporary, { recursive: true, force: true });
  }
}

async function main() {
  const args = process.argv.slice(2);
  const sourceIndex = args.indexOf("--source");
  const source =
    sourceIndex >= 0
      ? args[sourceIndex + 1]
      : process.env.POPULAR_SKILLS_SOURCE;
  if (sourceIndex >= 0 && !source)
    throw new Error("--source requires a JSON file");
  const entries = JSON.parse(
    await readFile(
      source ??
        path.resolve(repoRoot, "../magpie/internal/library/market_skills.json"),
      "utf8",
    ),
  );
  const skills = selectPopularSkills(entries);
  let missingDescriptions = 0;
  await concurrent(skills, async (skill) => {
    try {
      const description = metaDescription(
        await (
          await request(`https://skills.sh/${skill.source}/${skill.skillId}`)
        ).text(),
      );
      if (description) skill.description = description;
      else missingDescriptions++;
    } catch {
      missingDescriptions++;
    }
  });
  const owners = [
    ...new Set(skills.map((skill) => skill.source.split("/")[0].toLowerCase())),
  ].sort();
  const ownerDir = path.join(repoRoot, "src/assets/library-logos/owners");
  await mkdir(ownerDir, { recursive: true });
  await concurrent(owners, async (owner) => {
    const bytes = Buffer.from(
      await (
        await request(`https://github.com/${owner}.png?size=96`)
      ).arrayBuffer(),
    );
    await writeFile(
      path.join(ownerDir, `${owner}.png`),
      await avatarPng(bytes),
    );
  });
  const generatedAt = new Date().toISOString();
  await writeFile(
    path.join(repoRoot, "src/components/library/popularSkills.json"),
    JSON.stringify({ source: sourceLabel, generatedAt, skills }, null, 2) +
      "\n",
  );
  const noticePath = path.join(repoRoot, "src/assets/library-logos/NOTICE.md");
  const marker = "## Popular skills snapshot";
  const notice = (await readFile(noticePath, "utf8"))
    .split(marker)[0]
    .trimEnd();
  await writeFile(
    noticePath,
    `${notice}\n\n${marker}\n\nMetadata adapted from [magpie](https://github.com/yetone/magpie), \`internal/library/market_skills.json\` (MIT, © 2026 yetone; full license above). Snapshot generated ${generatedAt}. Only catalogue metadata is bundled; no third-party skill bodies or scripts are copied. The official field records upstream publisher metadata and does not represent a security audit. English descriptions were retrieved once from the corresponding skills.sh metadata pages; unavailable descriptions are omitted.\n\nGitHub owner avatars, used to identify their respective publishers (rights remain with their owners):\n\n${owners.map((owner) => `- \`owners/${owner}.png\` — https://github.com/${owner}.png?size=96`).join("\n")}\n\nAvatars retrieved at generation time and bundled locally. No runtime image or description requests are made.\n`,
  );
  console.log(
    JSON.stringify({
      skills: skills.length,
      owners: owners.length,
      missingDescriptions,
      generatedAt,
    }),
  );
}

if (
  process.argv[1] &&
  import.meta.url === pathToFileURL(path.resolve(process.argv[1])).href
)
  await main();
