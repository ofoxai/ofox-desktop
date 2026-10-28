import { describe, expect, it } from "vitest";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { ASSISTANT_ERROR_MESSAGES } from "@/lib/api/assistant";

function strings(tree: unknown, path = ""): [string, string][] {
  if (typeof tree === "string") return [[path, tree]];
  if (tree && typeof tree === "object")
    return Object.entries(tree).flatMap(([key, value]) =>
      strings(value, path ? `${path}.${key}` : key),
    );
  return [];
}

describe("the assistant's wording", () => {
  it("calls a chat a 对话 in Chinese, as the rest of the assistant does", () => {
    expect(
      strings(zh.assistant).filter(([, text]) => text.includes("会话")),
    ).toEqual([]);
  });

  it("calls earlier messages メッセージの以前のバージョン in Japanese", () => {
    expect(
      strings(ja.assistant).filter(([, text]) => text.includes("メッセージ版")),
    ).toEqual([]);
  });

  it.each([
    ["en", en],
    ["zh", zh],
    ["ja", ja],
  ])("has a message in %s for every error code it maps", (_, locale) => {
    const errors = locale.assistant.errors as Record<string, string>;
    const missing = [
      ...new Set(Object.values(ASSISTANT_ERROR_MESSAGES)),
    ].filter((key) => !errors[key]);
    expect(missing).toEqual([]);
  });

  it("says why a backup or restore can't start now, in each language", () => {
    expect(ASSISTANT_ERROR_MESSAGES.assistant_transfer_busy).toBe(
      "transferBusy",
    );
    expect(zh.assistant.errors.transferBusy).toBe(
      "有回复或文件导入正在进行，结束后再备份或恢复。",
    );
  });
});
