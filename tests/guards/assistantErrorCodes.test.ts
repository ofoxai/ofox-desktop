// @vitest-environment node
import { readFileSync, readdirSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { ASSISTANT_ERROR_MESSAGES } from "@/lib/api/assistant";

function productionRustPaths(names: string[]) {
  return names
    .map((name) => name.replaceAll("\\", "/"))
    .filter((name) => {
      const filename = name.slice(name.lastIndexOf("/") + 1);
      return (
        filename.endsWith(".rs") &&
        filename !== "tests.rs" &&
        !filename.endsWith("_tests.rs") &&
        !name.split("/").includes("tests")
      );
    });
}

const SOURCES = [
  ...productionRustPaths(
    readdirSync(
      new URL("../../src-tauri/src/services/assistant/", import.meta.url),
      { recursive: true, encoding: "utf8" },
    ),
  ).map((name) => `../../src-tauri/src/services/assistant/${name}`),
  "../../src-tauri/src/commands/assistant.rs",
  "../../src-tauri/src/commands/assistant_local.rs",
  "../../assistant-runtime/src/local-task.mjs",
  "../../assistant-runtime/src/runtime.mjs",
];

/** Codes the host uses only inside itself; none reaches the renderer. */
const HOST_ONLY = new Set([
  // A refused key is retried with a new one inside the run.
  "assistant_retry_with_new_key",
  // The host's reply to a runtime tool call.
  "assistant_unknown_tool",
]);

/** Codes the renderer raises itself. */
const RENDERER_CODES = [
  "assistant_microphone_denied",
  "assistant_microphone_unavailable",
  "assistant_recording_interrupted",
  "assistant_audio_empty",
  "assistant_session_loading",
  "assistant_request_expired",
];

function productionSource(source: string) {
  // An external module at EOF leaves no test body here. Keep preceding production,
  // including code after cfg(test) helpers; retain existing inline-module handling.
  const externalTests = /(?:^|\r?\n)#\[cfg\(test\)\]\r?\nmod tests;\s*$/.exec(
    source,
  );
  if (externalTests) return source.slice(0, externalTests.index);
  const inlineTests = /(?:^|\r?\n)\s*#\[cfg\(test\)\]\s*mod tests\s*\{/.exec(
    source,
  );
  return inlineTests ? source.slice(0, inlineTests.index) : source;
}

function literalErrorCodes(source: string) {
  return [...source.matchAll(/"(assistant_[a-z0-9]+(?:_[a-z0-9]+)*)"/g)].map(
    ([, code]) => code,
  );
}

/** Every `assistant_*` string literal outside the test modules. */
function backendCodes() {
  const codes = new Set<string>();
  for (const path of SOURCES) {
    const source = readFileSync(new URL(path, import.meta.url), "utf8");
    const production = productionSource(source);
    for (const code of literalErrorCodes(production))
      if (!HOST_ONLY.has(code)) codes.add(code);
  }
  return codes;
}

describe("assistant error code parity", () => {
  it("discovers complete error codes while leaving diagnostic prefixes internal", () => {
    expect(
      literalErrorCodes(
        [
          'error.strip_prefix("assistant_document_sandbox_win32_");',
          'format!("assistant_document_sandbox_win32_{:08x}_at_{line}");',
          '"assistant_document__sandbox";',
          'fail("assistant_document_sandbox_unproven_runtime");',
        ].join("\n"),
      ),
    ).toEqual(["assistant_document_sandbox_unproven_runtime"]);
  });

  it.each(["\n", "\r\n"])(
    "keeps production after test helpers before an external test module (%j)",
    (newline) => {
      const source = [
        "#[cfg(test)]",
        "fn helper() {}",
        'fn production() { "assistant_still_production"; }',
        "",
        "#[cfg(test)]",
        "mod tests;",
        "",
      ].join(newline);

      expect(productionSource(source)).toContain(
        '"assistant_still_production"',
      );
      expect(productionSource(source)).not.toContain("mod tests;");
    },
  );

  it.each(["/", "\\"])(
    "discovers production Rust paths with %s separators without standalone tests",
    (separator) => {
      const paths = [
        "documents/windows_sandbox.rs",
        "documents/tests.rs",
        "host/local_interruption_tests.rs",
        "tests/fixture.rs",
        "documents/tests/fixture.rs",
        "tests.rs",
        "latest.rs",
        "test_support.rs",
        "documents/testimonials.rs",
        "tests_helpers/runner.rs",
        "test.rs",
        "documents/readme.md",
      ].map((name) => name.replaceAll("/", separator));

      expect(productionRustPaths(paths)).toEqual([
        "documents/windows_sandbox.rs",
        "latest.rs",
        "test_support.rs",
        "documents/testimonials.rs",
        "tests_helpers/runner.rs",
        "test.rs",
      ]);
    },
  );

  it("keeps production after conditional fields and helpers before inline tests", () => {
    const source = [
      "struct Host {",
      "    #[cfg(test)]",
      "    barrier: (),",
      "}",
      "#[cfg(test)]",
      "fn helper() {}",
      'fn production() { "assistant_still_production"; }',
      "#[cfg(test)]",
      'mod tests { fn fixture() { "assistant_fixture_only"; } }',
    ].join("\n");
    expect(literalErrorCodes(productionSource(source))).toEqual([
      "assistant_still_production",
    ]);
  });

  it("maps exactly the codes the backend can send, plus the renderer's own", () => {
    const backend = backendCodes();
    expect(backend.size).toBeGreaterThan(40);
    expect(Object.keys(ASSISTANT_ERROR_MESSAGES).sort()).toEqual(
      [...backend, ...RENDERER_CODES].sort(),
    );
  });
});
