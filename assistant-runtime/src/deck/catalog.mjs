import { readFileSync } from "node:fs";

// Bundled, pinned public resource metadata. Never take a path or URL from
// model arguments; discovery does not evaluate the vendored HTML/scripts.
export const PRESENTATION_CATALOG = JSON.parse(
  readFileSync(
    new URL(
      "../../skills/ofox-office-pptx/resources/catalog.json",
      import.meta.url,
    ),
    "utf8",
  ),
);
export const ADAPTED_STYLES = PRESENTATION_CATALOG.styles.filter(
  (style) =>
    style.status === "enabled" &&
    style.enabled &&
    style.adapted &&
    style.licenseReviewed &&
    style.theme,
);
