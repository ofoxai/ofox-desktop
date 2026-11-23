// @vitest-environment node
import { readFileSync } from "node:fs";
import { expect, it } from "vitest";

it("captures OfficeCLI's on-demand page SVG with the workspace limit on both platforms", () => {
  const host = readFileSync(
    new URL(
      "../../src-tauri/src/services/assistant/file_preview.rs",
      import.meta.url,
    ),
    "utf8",
  );
  const pageRenderer = host
    .split("pub async fn file_preview_page(")[1]
    .split("let input = json!")[0];
  const [windows, otherPlatforms] = pageRenderer.split("#[cfg(not(windows))]");
  const outputLimit = /Duration::from_secs\(60\),\s*([^,]+),\s*\)\s*\.await/;

  expect(windows.match(outputLimit)?.[1]).toBe("FILE_PAGE_SVG_LIMIT");
  expect(otherPlatforms.match(outputLimit)?.[1]).toBe("FILE_PAGE_SVG_LIMIT");
});
