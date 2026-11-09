# README media

Light and dark product screenshots use the real Ofox Desktop 1.5.0 React components and
fictional in-memory data. Static screenshots preserve 2880×1800 pixels from a
1440×900 CSS viewport at DPR 2 via lossless PNG → WebP. They contain no real account, device, configuration,
session history, model request, or bill. The assistant response is a written
example, not a benchmark. The tour is an edited four-scene illustration.

- `hero.svg`: original animated cover, embedding the actual English tools view.
- `hero-preview.webp`: still cover for viewers without SVG support.
- `tools-en.webp`, `tools-zh.webp`, `tools-ja.webp`: localized tool overviews.
- `sessions.webp`, `library.webp`, `discover.webp`, `assistant.webp`, `usage.webp`:
  the shared English scene gallery.
- `assistant-media.svg`: original animated image/video/speech capability illustration.
- `assistant-image.webp`, `assistant-video.webp`, `assistant-speech.webp`: real
  generation parameter controls with fictional catalog entries and an unsent draft.
- `skills.webp`: actual Skills distribution controls with synthetic library entries.
- `tour.gif`, `tour-poster.webp`, `tour.mp4`: eight-second tour, full-resolution still, and HD video.
- `tool-wall.svg`, `data-flow.svg`: original tool and bound-request diagrams.

- `binding-flow.svg`, `configuration-rescue.svg`: five-stage animated workflow
  explanations with reduced-motion support.
- `onboarding.gif`, `onboarding-poster.webp`, `onboarding.mp4`: actual discovery,
  choice and completion components with synthetic UI state; the button never
  invokes real binding. The compact 6.8-second video reaches setup completion within its first second; it then shows actual MCP preview and Skills configuration controls; it uses H.264 with fast start.

GIFs are 1200×750 at 12 fps; MP4s are 1440×900 at 30 fps. Short dissolves, gentle
camera motion, and a progress line give the edited loops a faster rhythm.

Every media file has a `-light` counterpart immediately before its extension.
The README selects light assets through `<picture>` and preserves dark fallbacks.
The onboarding and workspace tours are edited loops with dissolves, rather than
recordings of live installation, account access or model requests.

Reproduction, safety boundaries and checks: [showcase guide](../../scripts/readme-showcase.md).

## Credits

Ofox uses its existing application mark. Tool marks retain their respective
owners' rights. The SVG wall embeds the same selected marks used in the product:
see [brand sources](../../docs/brand-assets.md), [agent icon notices and licenses](../../src/assets/agent-logos/NOTICE.md),
and [MCP icon notices](../../src/assets/library-logos/NOTICE.md). Existing license
texts remain in their source directories. No endorsement is implied.

Original layout, diagrams and fixture code are covered by the repository's
[MIT license](../../LICENSE).

The README embeds the primary onboarding GIF once. Workspace-tour videos and
static covers remain available as media files; duplicate GIFs and still covers
are not embedded in the README.
