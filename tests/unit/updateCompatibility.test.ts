import { describe, expect, it } from "vitest";
import { updateCompatibilityIssue } from "@/lib/updateCompatibility";

describe("update compatibility feedback", () => {
  it("recognizes the minimum OS from native string and Error rejections", () => {
    for (const error of [
      "updateRequiresMacOS:14.0",
      new Error("updateRequiresMacOS:14.0"),
    ]) {
      expect(updateCompatibilityIssue(error)).toEqual({
        key: "update.requiresMacOS",
        version: "14.0",
      });
    }
  });
  it.each(["updateCompatibilityUnknown", "invalidUpdateManifest"])(
    "reports unverifiable compatibility: %s",
    (code) => {
      expect(updateCompatibilityIssue(code)).toEqual({
        key: "update.compatibilityUnknown",
      });
    },
  );
  it.each([
    "network timeout",
    "updateRequiresMacOS:14.0 https://untrusted.example/",
    "updateRequiresMacOS:<script>",
    null,
  ])("leaves unrelated errors on the existing generic path", (error) => {
    expect(updateCompatibilityIssue(error)).toBeNull();
  });
});
