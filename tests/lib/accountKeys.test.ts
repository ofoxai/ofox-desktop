import { beforeAll, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { announceKeyReconcile, announceLogout } from "@/lib/accountKeys";

const toast = vi.hoisted(() => ({
  success: vi.fn(),
  warning: vi.fn(),
}));
vi.mock("sonner", () => ({ toast }));

beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
});

const t = i18n.t.bind(i18n);

describe("logging out", () => {
  it("says the bound tools are paused until the next sign-in", () => {
    toast.success.mockClear();
    announceLogout({ stopped: ["claude", "codex"], notRevoked: [] }, t);
    expect(toast.success).toHaveBeenCalledWith(
      "已退出登录。已绑定的工具暂停使用，重新登录后自动恢复。",
    );
  });

  it("says a key not stopped on the server is deleted at the next sign-in", () => {
    toast.warning.mockClear();
    announceLogout({ stopped: ["claude", "codex"], notRevoked: ["codex"] }, t);
    expect(toast.warning).toHaveBeenCalledWith(
      "Codex 的 API Key 没能在服务器上停用（可能没联网），暂时还会用这个账号的额度。下次用这个账号在本机登录时会自动删除，也可以现在去 Ofox 控制台删除。",
      expect.anything(),
    );
  });

  it("keeps the plain message when nothing was bound", () => {
    toast.success.mockClear();
    announceLogout({ stopped: [], notRevoked: [] }, t);
    expect(toast.success).toHaveBeenCalledWith("已退出登录");
  });
});

describe("signing in", () => {
  it("names the tools that now use this account's balance", () => {
    toast.success.mockClear();
    announceKeyReconcile(
      { switched: ["claude", "gemini"], failed: [] },
      t,
      "bob@example.com",
    );
    expect(toast.success).toHaveBeenCalledWith(
      "Claude Code、Gemini 已改用 bob@example.com 的额度。",
    );
  });

  it("explains a tool it could not switch", () => {
    toast.warning.mockClear();
    announceKeyReconcile(
      {
        switched: [],
        failed: [
          { tool: "opencode", message: "接入配置已被修改，没有自动更换。" },
        ],
      },
      t,
      null,
    );
    expect(toast.warning).toHaveBeenCalledWith(
      "OpenCode 还在用之前账号的 API Key：接入配置已被修改，没有自动更换。",
      expect.anything(),
    );
  });

  it("stays quiet when every tool already uses this account", () => {
    toast.success.mockClear();
    toast.warning.mockClear();
    announceKeyReconcile({ switched: [], failed: [] }, t, "bob@example.com");
    expect(toast.success).not.toHaveBeenCalled();
    expect(toast.warning).not.toHaveBeenCalled();
  });
});
