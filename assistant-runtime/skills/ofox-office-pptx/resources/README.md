# Public PPT style resource intake

`catalog.json` records 24 distinct candidate source identities and their pinned provenance. These are source references, not 24 adapted or validated PPTX templates. The five representative IDs are `blue-professional`, `cartesian`, `course-module`, `product-launch`, and `architectural-plan`.

## Files and provenance

- `vendor/beautiful-html-templates/`: original selected HTML, JSON, design notes and adjacent JavaScript from `zarazhangrui/beautiful-html-templates@e5e204fb1f3b06290846e7dcd7aceddabeceec8c`; retain its MIT `LICENSE`.
- `vendor/open-design-html-ppt/`: original selected full-deck HTML/CSS, documentation and supporting code from the MIT `design-templates/html-ppt` subtree of `nexu-io/open-design@07a86b1041531bfa0bec1006d9c0993985fee560`; retain the subtree `LICENSE`. The package declares `lewislulu/html-ppt-skill` upstream; no upstream commit equivalence is claimed.
- `vendor/officecli/`: original `dark--architectural-plan/style.md` and `build.sh` from `iOfficeAI/OfficeCLI@c56512c8c7b76fe284f09940e4cd492470099d3e`; retain its Apache-2.0 `LICENSE` and `NOTICE`. The binary sample PPTX is excluded.

The upstream directory hierarchy is retained so the selected HTML files' local references resolve without editing their original bytes. Shared OpenDesign theme CSS and animation modules are supporting package code, not additional candidate styles. The upstream beautiful `index.json` inventories 34 upstream templates; the Ofox candidate roster is exclusively `catalog.json`.

Every imported upstream file has its source path and original SHA-256 in the catalog. Source files were compared to the pinned Git tree using their Git blob SHA-1, as an additional byte-identity check. Ofox metadata and this README are new; imported upstream files have no modifications. No private ofox-agent files were used.

## Status and asset boundary

At source intake all 24 entries have `status: candidate`, `licenseReviewed: true` for repository-owned code/text, and `adapted`, `verified`, `enabled` false. Later adapter work must update its own state and evidence; code-license review never implies font or image clearance, runtime compatibility, visual quality, editable objects, or macOS/Windows PowerPoint acceptance.

The current catalog enables five representatives experimentally, with native themes, composition recipes and documented coverage. They remain `verified: false`; original fonts are intentionally replaced by system-font roles and require target-OS inspection. The remaining 19 sources are candidates. Selection does not execute the original HTML or scripts, and the adapters are not complete HTML-to-PPTX conversions. Quality expansion is gated by the separate equal-content experiment and editing acceptance.

`externalDependencies` conservatively inventories declared font URLs, other URLs and font families in the style plus supporting package sources. This includes design-note examples and unused optional theme declarations; it is not a measurement of fonts actually rendered. Fonts, images, logos and brands require separate review. No fonts or image binaries were downloaded. CJK glyph coverage and substitution remain unverified.

Some original HTML includes external Google Fonts and unpinned JavaScript CDN references (for example Cartesian's Chart.js). These remain unchanged reference sources. No external dependency was fetched, no upstream script was executed, and no source HTML was launched during intake. Runtime adapters should use their own reviewed local/native objects instead of executing arbitrary upstream build or browser scripts.

These sources contain fictional/example text and branded references. Benchmarks must use Ofox's independent synthetic content, not inherit upstream example claims or metrics.
