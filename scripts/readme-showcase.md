# README showcase workspace

This separate browser entry renders the real Ofox Desktop 1.5.0 Shell and page
components with fictional in-memory data. It is excluded from the production
entry and never creates an account, calls a model, reads native sessions, or
writes an agent configuration. It illustrates the interface; it is not evidence
of a successful live request, native installation, or Windows verification.

## Run

```sh
pnpm install --frozen-lockfile
pnpm exec vite --config scripts/readme-showcase.vite.ts
```

Open `http://127.0.0.1:3026/scripts/readme-showcase.html?page=tools&lang=en`.
Use a fresh browser tab at this dedicated loopback origin. Keep the application
viewport at **1440×900 CSS pixels** and set device scale factor to **2**. Wait for
the intended synthetic content and local fonts to load. Capture **2880×1800 PNGs**
directly from the browser compositor; do not enlarge an existing 1440px image.
Wait for finite dialog entry animations to finish before capture so text is not
sampled during a scale transform. The generic browser screenshot preview returns
JPEG and is not the media master.
The original JPEG captures softened small text before WebP encoding; the media
pipeline now uses lossless PNG → WebP instead.

| Query   | Values                                                                                                                                                                           |
| ------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `page`  | `tools` (default), `sessions`, `library`, `assistant`, `usage`, `onboarding`                                                                                                     |
| `lang`  | `en` (default), `zh`, `ja`                                                                                                                                                       |
| `theme` | `dark` (default), `light`                                                                                                                                                        |
| `scene` | `profile` on tools; `discover`, `mcp-preview`, or `skills` on library; `conversation` or `media` on assistant; `discovery`, `complete`, `mcp-preview`, or `skills` on onboarding |

The production sidebar changes pages normally. On the assistant page select the
fictional **Aurora — architecture review** conversation to display its sample
Markdown response, or use `scene=conversation` to initialize that selection through
the production row's existing handler. On the tools page `scene=profile` opens the real model-profile
review dialog. Applying settings, launching tools, generating content, and other
unimplemented actions are deliberately rejected by the sandbox.

All screenshots should retain the small localized sample-data label. The
English main view also has Chinese and Japanese variants; the other English
scenes are shared by the translated README files with localized captions.

## Isolation and checks

The guard installs **before dynamic application imports**. Storage is replaced
with page-local memory without reading origin storage. Tauri IPC is dispatched
only to synthetic handlers; unknown commands reject. Fetch, XMLHttpRequest,
WebSocket, EventSource, Worker, sendBeacon, external window navigation, links,
forms and native asset URLs are blocked. The HTML Content Security Policy also
blocks connections and remote image/media resources. The Vite development
client may report a blocked connection; HMR is disabled in the fixture config.

`window.__README_SHOWCASE__` exposes safe diagnostics (`ready`, `calls`,
`blockedIpc`, `blockedNetwork`). It records operation names, never request bodies,
credentials or remote addresses. `ready` is set only after the React host commits;
also verify the target page is visibly loaded before capture.

```sh
node --test scripts/readme-showcase-guard.node.mjs
pnpm exec tsc --noEmit -p scripts/tsconfig.readme-showcase.json
```

The UI uses production components and the existing repository fixture approach:
synthetic tools/model snapshots and assistant conversation data in
`readme-showcase-data.ts`, plus synthetic library, session and usage responses in
`readme-pages.tsx`. No production endpoint, database schema, account boundary,
or configuration-writing API is changed.

Inspect all generated images and every animation frame before committing.
Do not introduce real names, balances, emails, credentials, machine names,
personal filesystem paths, internal hosts, or actual conversation content.

## Reproduce the committed media

Baseline: release `1.5.0_20261105_0159` (`2d7886ac22a48986ac9b7d6aff3d7a0749e9c5d8`),
with the same source tree as main `e744377c4bd93a09be82d7af399eff35a2cfc188`.
The fixture adapts this source only for isolated browser display. No native or
live integration is being certified by these images.

Prerequisites: the pinned pnpm dependencies, Python 3, `ffmpeg` with PNG/GIF,
`xfade`, `palettegen` and `paletteuse`, plus `cwebp` from the WebP tools.
On macOS these tools are available with `brew install ffmpeg webp`.

Capture reviewed **2880×1800 PNGs** into a temporary directory. Keep the localized
sample-data badge visible. In CUA's documented CDP capability, use
`Emulation.setDeviceMetricsOverride` with `width: 1440`, `height: 900`,
`deviceScaleFactor: 2`, `mobile: false`, then `Page.captureScreenshot` with
`format: "png"`, `fromSurface: true`, `captureBeyondViewport: false`, and
`clip: { x: 0, y: 0, width: 1440, height: 900, scale: 1 }`. Decode its returned
base64 data directly to a `.png` file. Clear the emulation override when finished.
The PNG header must report 2880×1800; verify this rather than assuming a DPR.
Use:

| Filename                                       | Fixture view                                                   |
| ---------------------------------------------- | -------------------------------------------------------------- |
| `tools-en.png`, `tools-zh.png`, `tools-ja.png` | `page=tools` with matching `lang`                              |
| `sessions.png`                                 | `page=sessions`; wait for the synthetic session list           |
| `library.png`                                  | `page=library`; wait for MCP status to finish loading          |
| `discover.png`                                 | `page=library&scene=discover`                                  |
| `assistant.png`                                | `page=assistant&scene=conversation`                            |
| `usage.png`                                    | `page=usage`; wait for rankings, scroll to local-history trend |
| `profile.png`                                  | `page=tools&scene=profile`; preview only                       |
| `mcp-preview.png`                              | `page=library&scene=mcp-preview`; preview only                 |

The eight-second tour is an edited illustration with short dissolves, assembled
from tools, profile review, MCP review, and assistant captures. The compact
onboarding edit reaches its completion scene within its first second. Editorial
camera movement and a progress line begin immediately; the source screenshots
remain unchanged. These edits are not uncut recordings of native operations.

```sh
node scripts/readme-media.mjs /path/to/reviewed-captures
python3 scripts/readme-graphics.py
node scripts/check-readme-showcase.mjs
```

The media script strips metadata and preserves the full 2880×1800 source pixels
in lossless WebP screenshots and still posters. It creates a 1200×750 looping GIF
at 12 fps and a 1440×900 silent H.264 MP4 at 30 fps for each tour. Four-scene
workspace tours run eight seconds; the resource-configuration onboarding edit runs 6.8 seconds.
The SVG generator embeds the actual English tools capture and the repository's
existing brand marks. Its decorative animation honors reduced-motion settings;
the artwork is fully self-contained, with no scripts or remote resources.

For a static hero fallback, capture `assets/readme/hero.svg` at 1600×910 CSS
pixels with device scale factor 2, yielding `hero-preview.png` at 3200×1820,
then run:

```sh
cwebp -quiet -lossless -m 6 -metadata none hero-preview.png -o assets/readme/hero-preview.webp
```

Targets: each static asset under 1 MiB and the GIF under 8 MiB. These are size
budgets, not performance measurements. No JPEG recompression or upscaling is
used for the current static masters. The README presents the four gallery scenes
at full width and links both theme originals instead of shrinking full windows
into two columns. Sample timestamps change when the fixture is recaptured.

## Validation of this change

Checked on macOS against the 1.5.0 source baseline above:

- Production TypeScript, formatter and renderer build passed.
- All 236 frontend test files passed: 2,879 tests. The separate Node sandbox
  checks passed all six cases; their `.node.mjs` name avoids Vitest discovery.
- The independent fixture TypeScript and formatting checks passed.
- Media preflight passed three languages, eight shared section anchors and
  50 media files, including image dimensions, stripped WebP metadata, SVG XML,
  size budgets, four short GIFs, and four silent H.264 videos.
  All 16 light/dark image pairs per language resolve. Each README has six
  collapsed sections for generation parameters, FAQ, and development. The main
  setup GIF, application screenshots, and compatibility matrix are visible by
  default. Only the onboarding GIF is embedded; the workspace tour is a video link.
- Browser checks covered all three languages at 390px README width and at
  900px in both fixture themes, plus Tab / Shift+Tab / Escape in profile review.
- The official download page, latest GitHub Release and four badge URLs
  returned HTTP 200. All local documentation and image references resolve.
- All 24 full-resolution static WebP screenshots were verified pixel-identical
  to their PNG masters. Source screenshots and animation contact sheets were
  visually reviewed; the onboarding completion screen is visible by 0.8 seconds. These are browser fixture checks; native Windows, installers,
  live model calls and Rust backend behavior were not changed or revalidated.

## Light artwork and one-click setup motion

Repeat the scene captures with `theme=light`, keeping the same filenames in a
separate directory, then generate matching light assets:

```sh
node scripts/readme-media.mjs /path/to/reviewed-light-captures --theme light
python3 scripts/readme-graphics.py --theme light
python3 scripts/readme-flows.py
```

The README uses standard `<picture>` light sources with original dark fallbacks.
Binding and configuration-rescue diagrams have five moving stages, flowing
connectors and reduced-motion alternatives. They explain the workflow; they
are not fabricated product screens or evidence of live repairs.

The onboarding entry renders the actual tool discovery and setup-complete
screens. Its Connect button changes React demonstration state only: it never
calls the production binding function, requests a key, or writes configuration.
Opening the console clears the onboarding-only discovery response override.
The console is the shared fixed five-tool sample workspace. Exported movies
reselect all five tools before Connect, so their completion and console counts
match; other selections illustrate the completion screen only, not persistence.

For the onboarding movie, open `page=onboarding&scene=discovery&lang=en` with the
desired theme. Wait for **Connect (5) →**. Capture `discovery.png` after
deselecting Hermes, `choose.png` after selecting it again, `complete.png` after
clicking Connect, and `console.png` after clicking Open console. These real
component interactions are restricted to synthetic state. Use separate dark
and light capture directories. Also capture `mcp-preview.png` from
`page=library&scene=mcp-preview` and `skills.png` from `page=library&scene=skills`.
The real setup-complete screen only offers Open console. The edited sequence
continues through that console to the actual Library MCP and Skills controls;
it does not add fictional buttons to a production screen. The README provides
separate MCP/Skills guide buttons below the demo summary.

```sh
node scripts/readme-media.mjs /path/to/onboarding-dark --onboarding
node scripts/readme-media.mjs /path/to/onboarding-light --theme light --onboarding
```

Each command creates a 6.8-second, 1200×750 GIF and full-resolution still poster
plus a muted 1440×900 H.264 MP4. The edit removes the long discovery/selection
holds: it reaches completion within the first second, then opens the console.
The later scenes show MCP preview and Skills distribution. MP4 uses `+faststart`. FFmpeg needs its `libx264` encoder.
GIFs stay below 8 MiB and videos below 12 MiB. The edited loop dissolves back to
the first scene. No microphone, device audio or live desktop was recorded.

Capture each hero SVG at 1600×910 CSS pixels / DPR 2 after regenerating it, then use
`cwebp -quiet -lossless -m 6 -metadata none` to save dark and light stills as
`hero-preview.webp` and `hero-preview-light.webp`. All captured product pages
retain the sample-data badge.

## Assistant generation controls

`page=assistant&scene=media&lang=en&theme=light` opens the fictional **Studio —
image, video, and voice** draft through the production conversation selection
handler. Open each existing Generate image / Generate video / Generate speech
chip to capture its real parameter popover. Use the same 1440×900 CSS viewport
at DPR 2 and wait for finite entry animations. Save `assistant-image.png`,
`assistant-video.png`, and `assistant-speech.png`, then encode each with
`cwebp -quiet -lossless -m 6 -metadata none`. Repeat in the dark theme.

The catalog entries Demo Image, Demo Video, and Demo Speech are synthetic;
prices are unknown and their parameters are only interface examples. The fixture
allows draft-only updates on `readme-media-session` in page-local memory. It
rejects other session IDs, permission changes, sending, quoting, and generation.
No generated output or successful model request is claimed. `assistant-media.svg`
is an original conceptual illustration, separate from the real UI screenshots.
The three parameter screenshots are collapsed by default in each README.

The guide buttons point to the six explicit language/resource anchors in
`docs/resource-setup.md`. Detailed upstream code provenance is retained in
`docs/acknowledgements.md`; the original repository and module notices stay intact.
