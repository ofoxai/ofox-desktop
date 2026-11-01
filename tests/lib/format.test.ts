import { describe, expect, it } from "vitest";
import { fmtBinaryBytes } from "@/lib/format";

describe("binary byte sizes", () => {
  it("reads 1024 bytes as a KB, one decimal only below 10", () => {
    expect(
      [999, 2048, 3_400_000, 26_214_400, 1_610_612_736].map((value) =>
        fmtBinaryBytes(value),
      ),
    ).toEqual(["999 B", "2.0 KB", "3.2 MB", "25 MB", "1.5 GB"]);
  });
});
