import { beforeEach, describe, expect, it } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { launchErrorMessage } from "@/lib/launchErrors";

beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

describe("launchErrorMessage", () => {
  it("names the missing tool so the user knows to install it", () => {
    expect(launchErrorMessage("TOOL_NOT_INSTALLED|codex", i18n.t)).toBe(
      "未找到 Codex，请先安装或检查安装路径",
    );
  });

  it("falls back to the tool id for a tool it has no label for", () => {
    expect(launchErrorMessage("TOOL_NOT_INSTALLED|newtool", i18n.t)).toBe(
      "未找到 newtool，请先安装或检查安装路径",
    );
  });

  it("shows the unavailable proxy endpoint", () => {
    expect(
      launchErrorMessage("LOCAL_PROXY_UNAVAILABLE|127.0.0.1:7890", i18n.t),
    ).toBe("本机代理 127.0.0.1:7890 未运行。请启动代理或恢复端口转发后重试。");
  });

  it("uses the generic message for anything else", () => {
    expect(launchErrorMessage(new Error("boom"), i18n.t)).toBe("打开工具失败");
  });
});
