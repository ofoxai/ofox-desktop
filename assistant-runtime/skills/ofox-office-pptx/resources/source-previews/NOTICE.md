# Actual upstream source previews

These JPEGs show the actual pinned upstream example pages, separately from the
independently authored eight-page Ofox examples under `src/assets/presentation-styles`.
They are raster previews, not native adapter coverage or generated user files.

| Source style       | Actual source pages | Rendered source pages |
| ------------------ | ------------------: | --------------------: |
| blue-professional  |                  10 |                    10 |
| cartesian          |                  10 |                    10 |
| course-module      |                   7 |                     7 |
| product-launch     |                   8 |                     8 |
| architectural-plan |                   6 |                     6 |

`navy` is Ofox's built-in master with zero content pages and three layouts, not an
upstream example deck. The other nineteen catalog identities remain candidates;
their source inventory does not imply native availability or rendered previews.

HTML was rendered by an existing Chrome in a new private headless profile. Only
the task-owned loopback source server was reachable. Original HTML/CSS bytes were
not edited; the current original page was selected, animations paused and navigation
chrome hidden for capture. Remote fonts were blocked and local system fallbacks
were used. The previews are not exact original-font or cross-platform fidelity proof.

Cartesian's original unversioned Chart.js URL was mapped to reviewed local
Chart.js **4.6.1**, with the bundled `@kurkle/color` **0.3.2** license retained.
This establishes rendering under that fixed current dependency environment, not
the version used by the original author. See `../preview-dependencies.json` and
its separate MIT license files. No npm package was installed. This dependency
is used by the isolated source-preview renderer, not by the document runtime.

The architectural source's exact `build.sh` was copied to an isolated task folder
and rebuilt with OfficeCLI 1.0.160. Its six original native pages were rasterized;
`rebuilt-original.pptx` records that rebuild, not an original upstream binary or
an Ofox user document. **The raw source rendering has clipped card text and
overlapping/clipped lower statistics and footers.** These findings are retained
in `../source-preview-evidence.json`; a zero schema-error count is not visual acceptance.

Original sample names, brands, claims and testimonials remain upstream examples.
They are not Ofox endorsements, independently established facts or user content.
Code/text licenses do not establish third-party font, image or trademark clearance.

Source versions, unchanged original hashes and retained notices:

- `beautiful-html-templates@e5e204fb1f3b06290846e7dcd7aceddabeceec8c`:
  blue-professional and cartesian; MIT, copyright 2026 Zara Zhang.
- OpenDesign `html-ppt@07a86b1041531bfa0bec1006d9c0993985fee560`:
  course-module and product-launch; subtree MIT, copyright 2026 lewis.
- `OfficeCLI@c56512c8c7b76fe284f09940e4cd492470099d3e`:
  architectural-plan; Apache-2.0, copyright 2026 OfficeCLI. Its LICENSE and NOTICE
  remain under `../vendor/officecli/`.

`../source-coverage.json` separates original sources, partial native adaptations,
independent Ofox examples and runtime user files. `../source-preview-evidence.json`
records the exact JPEG hashes, source input hashes, environment and known findings.
