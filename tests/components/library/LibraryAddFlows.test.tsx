import zhLocale from "@/i18n/locales/zh.json";
import enLocale from "@/i18n/locales/en.json";
import jaLocale from "@/i18n/locales/ja.json";
import { useState } from "react";
import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import { LibraryDiscover } from "@/components/library/LibraryDiscover";
import {
  LibraryMcpForm,
  mcpDraftFromJson,
  mcpDraftToEdits,
} from "@/components/library/LibraryMcpForm";
import {
  createMcpDraft,
  LIBRARY_RECOMMENDATIONS,
} from "@/components/library/libraryRecommendations";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";
import { LIBRARY_MCP_COPY } from "@/components/library/libraryMcpCopy";
import { LIBRARY_DISCOVER_COPY } from "@/components/library/libraryDiscoverCopy";
import { invoke } from "@tauri-apps/api/core";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

beforeEach(() => {
  for (const [lang, copy] of Object.entries({
    zh: zhLocale,
    en: enLocale,
    ja: jaLocale,
  }))
    i18n.addResourceBundle(lang, "translation", copy, true, true);
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_DISCOVER_COPY.zh },
    true,
    true,
  );
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_ADD_COPY.zh },
    true,
    true,
  );
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_MCP_COPY.zh },
    true,
    true,
  );
});

describe("library recommendations", () => {
  it("browses locally and selects one skill path without installing the repository", async () => {
    const choose = vi.fn();
    render(<LibraryDiscover kind="skills" onChoose={choose} />);
    expect(choose).not.toHaveBeenCalled();
    expect(invoke).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "选择 frontend-design" }),
    );
    expect(choose).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "skills",
        repository: {
          owner: "anthropics",
          name: "skills",
          branch: "main",
          directory: "skills/frontend-design",
        },
      }),
      expect.any(HTMLButtonElement),
    );
    expect(invoke).not.toHaveBeenCalled();
  });

  it("keeps reference examples explicit and Context7 credentials out of the recipe", async () => {
    render(<LibraryDiscover kind="mcp" onChoose={vi.fn()} />);
    for (const name of ["Filesystem", "Fetch"]) {
      const card = screen.getByRole("article", { name });
      await userEvent.click(
        within(card).getByRole("button", { name: "配置详情" }),
      );
      expect(within(card).getByText("官方参考示例")).toBeVisible();
    }
    const context = LIBRARY_RECOMMENDATIONS.find(
      (recipe) => recipe.id === "context7",
    );
    expect(context?.kind).toBe("mcp");
    if (context?.kind !== "mcp") throw new Error("missing Context7 recipe");
    expect(context.initialDraft.url).toBe("https://mcp.context7.com/mcp");
    expect(context.initialDraft.headers).toEqual([]);
    expect(JSON.stringify(context.initialDraft)).not.toContain("YOUR_API_KEY");
  });

  it("offers three editable instruction templates with localized content", async () => {
    const choose = vi.fn();
    render(<LibraryDiscover kind="instructions" onChoose={choose} />);
    const buttons = screen.getAllByRole("button", { name: /^选择 / });
    expect(buttons).toHaveLength(3);
    await userEvent.click(buttons[0]);
    const recipe = choose.mock.calls[0][0];
    expect(recipe.kind).toBe("instructions");
    for (const language of ["zh", "en", "ja"]) {
      expect(recipe.contentByLanguage[language]).toContain("\n");
      expect(recipe.contentByLanguage[language].length).toBeGreaterThan(40);
    }
  });
});

describe("friendly MCP form", () => {
  it("does not turn non-finite JSON values into null or accept credentials in a query URL", () => {
    expect(() =>
      mcpDraftFromJson('{"command":"server","timeout":1e400}'),
    ).toThrow(/^invalidJson$/);
    expect(() =>
      mcpDraftToEdits(
        createMcpDraft({
          name: "Remote",
          transport: "http",
          url: "https://example.org/mcp?api_key=private-sentinel",
        }),
      ),
    ).toThrow(/^invalidUrl$/);
  });

  it("requires an explicit permitted folder and preserves spaces as one argument", () => {
    const recipe = LIBRARY_RECOMMENDATIONS.find(
      (item) => item.id === "filesystem",
    );
    if (recipe?.kind !== "mcp") throw new Error("missing filesystem recipe");
    expect(() => mcpDraftToEdits(recipe.initialDraft)).toThrow(
      /^missingDirectory$/,
    );
    const edits = mcpDraftToEdits({
      ...recipe.initialDraft,
      directory: "C:\\Users\\Example\\My Documents",
    });
    expect(edits.find((edit) => edit.path[0] === "args")?.value).toEqual([
      "-y",
      "@modelcontextprotocol/server-filesystem",
      "C:\\Users\\Example\\My Documents",
    ]);
  });
  it("preserves argument boundaries and real empty environment values", () => {
    const draft = createMcpDraft({
      name: "Files",
      command: "npx",
      argsText:
        "-y\n@modelcontextprotocol/server-filesystem\n/Users/example/My Documents",
      environment: [{ id: "empty", name: "OPTIONAL_VALUE", value: "" }],
    });
    const edits = mcpDraftToEdits(draft);
    expect(edits).toContainEqual({
      path: ["args"],
      action: "set",
      value: [
        "-y",
        "@modelcontextprotocol/server-filesystem",
        "/Users/example/My Documents",
      ],
    });
    expect(edits).toContainEqual({
      path: ["env", "OPTIONAL_VALUE"],
      action: "set",
      value: "",
    });
    expect(edits.every((edit) => edit.action === "set" && !edit.fieldRef)).toBe(
      true,
    );
  });

  it("rejects masked secrets and case-insensitive duplicate headers without echoing them", () => {
    const remote = createMcpDraft({
      name: "Remote",
      transport: "http",
      url: "https://example.org/mcp",
    });
    for (const value of ["[REDACTED]", "<redacted>", "••••••"]) {
      expect(() =>
        mcpDraftToEdits({
          ...remote,
          headers: [{ id: "h", name: "Authorization", value }],
        }),
      ).toThrow("maskedValue");
    }
    expect(() =>
      mcpDraftToEdits({
        ...remote,
        headers: [
          { id: "a", name: "Authorization", value: "Bearer private-sentinel" },
          { id: "b", name: "authorization", value: "other" },
        ],
      }),
    ).toThrow(/^duplicateHeader$/);
  });

  it("pastes one MCP JSON definition while retaining advanced fields and credentials exactly", () => {
    const draft = mcpDraftFromJson(
      JSON.stringify({
        mcpServers: {
          docs: {
            type: "http",
            url: "https://example.org/mcp",
            headers: { Authorization: "Bearer private-sentinel" },
            timeout: 5000,
          },
        },
      }),
    );
    expect(draft.name).toBe("docs");
    expect(mcpDraftToEdits(draft)).toContainEqual({
      path: ["timeout"],
      action: "set",
      value: 5000,
    });
    expect(mcpDraftToEdits(draft)).toContainEqual({
      path: ["headers", "Authorization"],
      action: "set",
      value: "Bearer private-sentinel",
    });
    expect(() =>
      mcpDraftFromJson(
        '{"mcpServers":{"a":{"command":"a"},"b":{"command":"b"}}}',
      ),
    ).toThrow(/^oneServer$/);
  });

  it("saves a draft separately from preview and does not submit on transport selection", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", command: "mcp-server" }),
      );
      return <LibraryMcpForm value={draft} onChange={setDraft} onSave={save} />;
    }
    render(<Fixture />);
    await userEvent.click(screen.getByRole("radio", { name: "远程 HTTP" }));
    expect(save).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("服务器 URL"), {
      target: { value: "https://example.org/mcp" },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "保存到我的资源" }),
    );
    expect(save).toHaveBeenLastCalledWith(
      expect.objectContaining({
        transport: "http",
        url: "https://example.org/mcp",
      }),
      "draft",
    );
    await userEvent.click(screen.getByRole("button", { name: "预览并应用" }));
    expect(save).toHaveBeenLastCalledWith(expect.anything(), "apply");
  });

  it("keeps secrets private in the form and names every keyboard action", async () => {
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({
          name: "Remote",
          transport: "http",
          url: "https://example.org/mcp",
        }),
      );
      return (
        <LibraryMcpForm value={draft} onChange={setDraft} onSave={vi.fn()} />
      );
    }
    render(<Fixture />);
    await userEvent.click(screen.getByRole("button", { name: "添加请求头" }));
    const fields = screen.getByRole("group", { name: "请求头" });
    expect(within(fields).getByLabelText("值 1")).toHaveAttribute(
      "type",
      "password",
    );
    expect(
      within(fields).getByRole("button", { name: "移除请求头 1" }),
    ).toBeVisible();
    expect(
      document.querySelector("select,input[type=checkbox],input[type=radio]"),
    ).toBeNull();
  });
});
