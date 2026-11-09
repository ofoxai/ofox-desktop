/** Compress reviewed CUA captures and compose short, visibly moving UI edits.
 * Usage: node scripts/readme-media.mjs CAPTURES [--theme light] [--onboarding] [--dry-run]
 * Prefer PNG captures (including 2880×1800 DPR-2 captures); legacy JPEGs work too.
 * Static WebP images and posters preserve the source dimensions without resampling.
 * GIF: 1200×750 / 12 fps. MP4: 1440×900 / 30 fps. Tour: 8 s. Onboarding: 6.8 s.
 * Motion is an editorial camera move plus a top progress line, not a live recording
 * or evidence of a successful native setup/request. No desktop/API access occurs.
 */
import { spawnSync } from "node:child_process";
import {
  copyFileSync,
  existsSync,
  mkdtempSync,
  rmSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const captures = process.argv[2] && path.resolve(process.argv[2]);
const themeArgument = process.argv.indexOf("--theme");
const theme = themeArgument === -1 ? "dark" : process.argv[themeArgument + 1];
if (theme !== "dark" && theme !== "light")
  throw new Error("Theme must be dark or light.");
const suffix = theme === "light" ? "-light" : "";
const onboarding = process.argv.includes("--onboarding");
const dryRun = process.argv.includes("--dry-run");
const stem = onboarding ? "onboarding" : "tour";
const duration = onboarding ? 6.8 : 8;
const movieFps = 30;
const gifFps = 12;
const transition = 0.22;
if (!captures || process.argv[2].startsWith("--"))
  throw new Error(
    "Pass the directory of reviewed sample-data PNG/JPEG captures.",
  );
const output = path.join(root, "assets/readme");

function run(program, args) {
  const result = spawnSync(program, args, { encoding: "utf8" });
  if (result.status !== 0)
    throw new Error(
      result.stderr || result.error?.message || `${program} failed`,
    );
  return result.stdout;
}
function ffmpeg(args) {
  return run("ffmpeg", ["-hide_banner", "-loglevel", "error", "-y", ...args]);
}
function capture(name) {
  for (const extension of ["png", "jpg", "jpeg"]) {
    const candidate = path.join(captures, `${name}.${extension}`);
    if (existsSync(candidate)) return candidate;
  }
  throw new Error(`Missing reviewed capture: ${name}.png (or legacy JPEG).`);
}
function dimensions(input) {
  const probe = JSON.parse(
    run("ffprobe", [
      "-v",
      "error",
      "-select_streams",
      "v:0",
      "-show_entries",
      "stream=width,height",
      "-of",
      "json",
      input,
    ]),
  );
  const { width, height } = probe.streams?.[0] ?? {};
  if (
    !width ||
    !height ||
    width < 1440 ||
    height < 900 ||
    width * 5 !== height * 8
  )
    throw new Error(
      "Captures must be 16:10 at 1440×900 or larger; recapture rather than upsample.",
    );
  return { width, height };
}
function webp(input, destination) {
  run("cwebp", [
    "-quiet",
    "-lossless",
    "-m",
    "6",
    "-metadata",
    "none",
    input,
    "-o",
    destination,
  ]);
}
const staticNames = onboarding
  ? []
  : [
      "tools-en",
      "tools-zh",
      "tools-ja",
      "sessions",
      "library",
      "discover",
      "assistant",
      "usage",
    ];
// Focus coordinates are editorial framing only; no cursor or simulated click is
// added. The full source images and their sample-data labels remain available.
const scenes = onboarding
  ? [
      { name: "choose", x: 0.52, y: 0.46, seconds: 0.65 },
      { name: "complete", x: 0.5, y: 0.46, seconds: 0.85 },
      { name: "console", x: 0.57, y: 0.3, seconds: 1.5 },
      { name: "mcp-preview", x: 0.55, y: 0.5, seconds: 1.9 },
      { name: "skills", x: 0.58, y: 0.4, seconds: 1.9 },
    ]
  : [
      { name: "tools-en", x: 0.6, y: 0.28, seconds: 2 },
      { name: "profile", x: 0.56, y: 0.44, seconds: 2 },
      { name: "mcp-preview", x: 0.55, y: 0.5, seconds: 2 },
      { name: "assistant", x: 0.64, y: 0.38, seconds: 2 },
    ];
const sources = new Map();
let elapsed = 0;
const transitionOffsets = scenes.map((scene) => {
  elapsed += scene.seconds;
  return (elapsed - transition).toFixed(3);
});
for (const name of new Set([
  ...staticNames,
  ...scenes.map((scene) => scene.name),
])) {
  const input = capture(name);
  sources.set(name, { input, ...dimensions(input) });
}
if (dryRun) {
  console.log(
    JSON.stringify(
      {
        theme,
        stem,
        durationSeconds: duration,
        staticImages: "source dimensions preserved (no resampling)",
        gif: { width: 1200, height: 750, fps: gifFps, targetMiB: 8 },
        mp4: { width: 1440, height: 900, fps: movieFps, targetMiB: 12 },
        transitionSeconds: transition,
        sceneTimeline: scenes.map(({ name, seconds }) => ({ name, seconds })),
        firstVisibleMotion: "first 30-fps frame; progress visible within 0.1 s",
        sources: [...sources].map(([name, source]) => ({
          name,
          width: source.width,
          height: source.height,
        })),
      },
      null,
      2,
    ),
  );
} else {
  const scratch = mkdtempSync(path.join(tmpdir(), "ofox-readme-media-"));
  try {
    for (const name of staticNames)
      webp(sources.get(name).input, path.join(output, `${name}${suffix}.webp`));

    // Short scenes, with a 220 ms dissolve between each. A final short
    // first-scene clip creates a continuous loop. Each camera settles within
    // 550 ms at 1.8% zoom, leaving the text stable while the progress line moves.
    const clips = [...scenes, scenes[0]];
    const inputs = clips.flatMap((scene, index) => [
      "-loop",
      "1",
      "-framerate",
      String(movieFps),
      "-t",
      String(
        index === scenes.length
          ? transition
          : index === 0
            ? scene.seconds
            : scene.seconds + transition,
      ),
      "-i",
      sources.get(scene.name).input,
    ]);
    const filters = clips.map((scene, index) => {
      const frames = Math.ceil(
        Math.min(
          index === scenes.length ? transition : scene.seconds + transition,
          0.55,
        ) * movieFps,
      );
      const zoom = `1+0.018*min(on/${frames},1)`;
      // Keep the source's bottom sample-data label fixed and uncropped while
      // the rest receives the small editorial camera move.
      return `[${index}:v]split[work${index}][raw${index}];[raw${index}]scale=1440:900:flags=lanczos,crop=1440:36:0:864[foot${index}];[work${index}]zoompan=z='${zoom}':x='(iw-iw/zoom)*${scene.x}':y='(ih-ih/zoom)*${scene.y}':d=1:s=1440x900:fps=${movieFps}[camera${index}];[camera${index}][foot${index}]overlay=x=0:y=864,settb=AVTB,setpts=PTS-STARTPTS,format=yuv444p[v${index}]`;
    });
    for (let index = 1; index < clips.length; index++) {
      const before = index === 1 ? "v0" : `x${index - 1}`;
      const offset = transitionOffsets[index - 1];
      filters.push(
        `[${before}][v${index}]xfade=transition=fade:duration=${transition}:offset=${offset}[x${index}]`,
      );
    }
    const accent = theme === "light" ? "0xb84c20" : "0xffa66a";
    const track = theme === "light" ? "0xdbe2ec" : "0x343a49";
    filters.push(
      `[x${clips.length - 1}]fps=${movieFps},drawbox=x=0:y=0:w=iw:h=4:color=${track}:t=fill[base]`,
      `color=c=${accent}:s=1440x4:r=${movieFps}:d=${duration},format=rgba[progress]`,
      `[base][progress]overlay=x='-overlay_w+main_w*t/${duration}':y=0:shortest=1,format=yuv420p[movie]`,
    );
    const master = path.join(scratch, `${stem}.mp4`);
    ffmpeg([
      ...inputs,
      "-filter_complex_threads",
      "1",
      "-filter_complex",
      filters.join(";"),
      "-map",
      "[movie]",
      "-t",
      String(duration),
      "-an",
      "-c:v",
      "libx264",
      "-preset",
      "medium",
      "-crf",
      "18",
      "-movflags",
      "+faststart",
      "-map_metadata",
      "-1",
      master,
    ]);
    copyFileSync(master, path.join(output, `${stem}${suffix}.mp4`));
    ffmpeg([
      "-i",
      master,
      "-filter_complex_threads",
      "1",
      "-filter_complex",
      `[0:v]fps=${gifFps},scale=1200:750:flags=lanczos,split[a][b];[a]palettegen=max_colors=192:stats_mode=diff[p];[b][p]paletteuse=dither=none[out]`,
      "-map",
      "[out]",
      "-t",
      String(duration),
      "-loop",
      "0",
      "-map_metadata",
      "-1",
      path.join(output, `${stem}${suffix}.gif`),
    ]);
    webp(
      sources.get(scenes[0].name).input,
      path.join(output, `${stem}-poster${suffix}.webp`),
    );
    const sizes = Object.fromEntries(
      ["gif", "mp4"].map((extension) => [
        extension,
        Number(
          (
            statSync(path.join(output, `${stem}${suffix}.${extension}`)).size /
            1048576
          ).toFixed(2),
        ),
      ]),
    );
    console.log(
      `Generated ${theme} ${stem}: ${duration}s; GIF 1200×750/${gifFps}fps (${sizes.gif}MiB); MP4 1440×900/${movieFps}fps (${sizes.mp4}MiB); source-size poster${staticNames.length ? ` and ${staticNames.length} source-size screenshots` : ""}. Editorial camera motion + progress line, not a live recording.`,
    );
    if (sizes.gif > 8 || sizes.mp4 > 12)
      console.warn(
        "Media exceeds the preferred size budget; inspect readability before reducing color count or frame rate.",
      );
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}
