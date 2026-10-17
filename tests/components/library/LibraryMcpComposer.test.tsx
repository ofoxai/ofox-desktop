import { useState } from "react";
import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import {
  LibraryMcpForm,
  mcpDraftFromJson,
  mcpDraftToEdits,
} from "@/components/library/LibraryMcpForm";
import { createMcpDraft } from "@/components/library/libraryRecommendations";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";
import { LIBRARY_MCP_COPY } from "@/components/library/libraryMcpCopy";
import {
  createMcpIdentifier,
  mcpDraftsFromJson,
  mcpEditDelta,
  parseMcpCommand,
} from "@/components/library/libraryMcpInput";
import type { McpDefinition } from "@/lib/api/library";

beforeEach(() => {
  Element.prototype.scrollIntoView = vi.fn();
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

describe("MCP composer lossless input regressions", () => {
  it("preserves a real empty argument instead of dropping it", () => {
    const draft = mcpDraftFromJson(
      '{"name":"server","command":"server","args":["before","","after"]}',
    );
    expect(
      mcpDraftToEdits(draft).find((edit) => edit.path[0] === "args")?.value,
    ).toEqual(["before", "", "after"]);
  });
  it("rejects duplicated configuration keys instead of silently keeping the last credential", () => {
    expect(() =>
      mcpDraftFromJson(
        '{"name":"server","command":"server","env":{"TOKEN":"first","TOKEN":"second"}}',
      ),
    ).toThrow("duplicateJsonKey");
  });
});

const existing: McpDefinition = {
  id: "existing-uuid",
  name: "Docs",
  revision: 2,
  transport: "http",
  credentialState: "stored",
  fields: [
    { path: ["type"], fieldRef: "type-ref", valueType: "string", exists: true },
    { path: ["url"], fieldRef: "url-ref", valueType: "string", exists: true },
    {
      path: ["headers", "Authorization"],
      fieldRef: "secret-ref",
      valueType: "string",
      exists: true,
    },
    {
      path: ["timeout"],
      fieldRef: "timeout-ref",
      valueType: "number",
      exists: true,
    },
  ],
  targets: [],
};

describe("MCP composer editing", () => {
  it("saves only an explicitly replaced field and never submits saved placeholders", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", transport: "http" }),
      );
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          initial={existing}
          onSave={vi.fn()}
          onSaveEdits={save}
        />
      );
    }
    render(<Fixture />);
    expect(screen.getByLabelText("配置标识")).toHaveValue("existing-uuid");
    fireEvent.change(screen.getByLabelText("服务器 URL"), {
      target: { value: "https://example.org/new" },
    });
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).toHaveBeenCalledWith(
      [
        {
          path: ["url"],
          fieldRef: "url-ref",
          action: "set",
          value: "https://example.org/new",
        },
      ],
      "draft",
    );
    expect(JSON.stringify(save.mock.calls)).not.toContain("已保存");
    expect(JSON.stringify(save.mock.calls)).not.toContain("secret-ref");
  });
  it("does not replace a saved credential when only the display name changes", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", transport: "http" }),
      );
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          initial={existing}
          onSave={vi.fn()}
          onSaveEdits={save}
        />
      );
    }
    render(<Fixture />);
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Renamed" },
    });
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).toHaveBeenCalledWith([], "draft");
  });
  it("requires a deliberate confirmation before clearing saved fields for a new transport", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", transport: "http" }),
      );
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          initial={existing}
          onSave={vi.fn()}
          onSaveEdits={save}
        />
      );
    }
    render(<Fixture />);
    await userEvent.click(screen.getByRole("radio", { name: "本地命令" }));
    fireEvent.change(screen.getByLabelText("命令"), {
      target: { value: "server" },
    });
    expect(screen.getByText(/以下已保存字段将清除/)).toHaveTextContent(
      "headers.Authorization",
    );
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("请确认连接方式变化");
    await userEvent.click(
      screen.getByRole("checkbox", { name: "我确认清除不兼容字段" }),
    );
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).toHaveBeenCalledWith(
      expect.arrayContaining([
        {
          path: ["headers", "Authorization"],
          fieldRef: "secret-ref",
          action: "clear",
        },
      ]),
      "draft",
    );
  });
  it("does not persist a previous valid argument array after the edited JSON becomes invalid", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", command: "server" }),
      );
      return <LibraryMcpForm value={draft} onChange={setDraft} onSave={save} />;
    }
    render(<Fixture />);
    await userEvent.click(screen.getByRole("radio", { name: "JSON 数组" }));
    fireEvent.change(screen.getByLabelText("参数"), {
      target: { value: '["valid"]' },
    });
    fireEvent.change(screen.getByLabelText("参数"), {
      target: { value: '["broken"' },
    });
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).not.toHaveBeenCalled();
  });
  it("does not auto-select targets and preserves a manually edited identifier", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", command: "server" }),
      );
      const [tools, setTools] = useState<string[]>([]);
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          onSave={save}
          targets={[
            {
              toolId: "codex",
              path: "",
              targetGroupId: "c",
              available: true,
              discoveryStatus: "available",
              status: "unassigned",
            },
          ]}
          selectedTools={tools}
          onToolsChange={setTools}
        />
      );
    }
    render(<Fixture />);
    expect(screen.getByRole("button", { name: "Codex" })).toHaveAttribute(
      "aria-pressed",
      "false",
    );
    expect(screen.getByRole("button", { name: "预览并应用" })).toBeDisabled();
    fireEvent.change(screen.getByLabelText("配置标识"), {
      target: { value: "chosen-id" },
    });
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "New name" },
    });
    expect(screen.getByLabelText("配置标识")).toHaveValue("chosen-id");
    await userEvent.click(screen.getByRole("button", { name: "Codex" }));
    await userEvent.click(screen.getByRole("button", { name: "预览并应用" }));
    expect(save).toHaveBeenCalledWith(
      expect.objectContaining({ name: "New name" }),
      "apply",
    );
  });
  it("requires selecting JSON entries and never imports unselected definitions", async () => {
    const many = vi.fn();
    render(
      <LibraryMcpForm
        value={createMcpDraft()}
        onChange={vi.fn()}
        onSave={vi.fn()}
        onImportMany={many}
      />,
    );
    await userEvent.click(screen.getByRole("radio", { name: "粘贴配置" }));
    fireEvent.change(screen.getByLabelText("MCP 配置 JSON"), {
      target: {
        value: '{"servers":{"a":{"command":"a"},"b":{"command":"b"}}}',
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "识别配置" }));
    expect(screen.getByRole("checkbox", { name: "选择 a" })).toHaveAttribute(
      "aria-checked",
      "false",
    );
    expect(
      screen.getByRole("button", { name: "保存所选到资源库" }),
    ).toBeDisabled();
    await userEvent.click(screen.getByRole("checkbox", { name: "选择 b" }));
    await userEvent.click(
      screen.getByRole("button", { name: "保存所选到资源库" }),
    );
    expect(many).toHaveBeenCalledWith(
      [expect.objectContaining({ name: "b", command: "b" })],
      "draft",
    );
  });
  it("fills a parsed command only after the program and argument review is confirmed", async () => {
    const change = vi.fn();
    render(
      <LibraryMcpForm
        value={createMcpDraft()}
        onChange={change}
        onSave={vi.fn()}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "粘贴完整命令" }));
    fireEvent.change(screen.getByLabelText("完整启动命令"), {
      target: { value: 'node "a b.js" ""' },
    });
    await userEvent.click(screen.getByRole("button", { name: "解析命令" }));
    expect(change).not.toHaveBeenCalled();
    await userEvent.click(screen.getByRole("button", { name: "确认填入表单" }));
    expect(change).toHaveBeenCalledWith(
      expect.objectContaining({ command: "node", args: ["a b.js", ""] }),
    );
  });
  it("blocks malformed new private pairs during editing rather than saving an earlier valid delta", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", transport: "http" }),
      );
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          initial={existing}
          onSave={vi.fn()}
          onSaveEdits={save}
        />
      );
    }
    render(<Fixture />);
    await userEvent.click(screen.getByRole("button", { name: "添加请求头" }));
    fireEvent.change(screen.getByLabelText("名称 1"), {
      target: { value: "invalid header" },
    });
    fireEvent.change(screen.getByLabelText("值 1"), {
      target: { value: "private-value" },
    });
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "有效的变量或请求头名称",
    );
    expect(screen.getByRole("alert")).not.toHaveTextContent("private-value");
  });
  it("supports keyboard save without emitting untouched values or submitting twice", async () => {
    let resolve: () => void = () => {};
    const save = vi.fn(
      () =>
        new Promise<void>((done) => {
          resolve = done;
        }),
    );
    render(
      <LibraryMcpForm
        value={createMcpDraft({ name: "Docs", transport: "http" })}
        onChange={vi.fn()}
        initial={existing}
        onSave={vi.fn()}
        onSaveEdits={save}
      />,
    );
    fireEvent.keyDown(screen.getByLabelText("名称"), {
      key: "s",
      ctrlKey: true,
    });
    fireEvent.keyDown(screen.getByLabelText("名称"), {
      key: "s",
      ctrlKey: true,
    });
    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith([], "draft");
    await act(async () => resolve());
  });
  it("keeps the visible edited argv when switching its action Keep → Replace", async () => {
    const save = vi.fn();
    const local = {
      ...existing,
      transport: "stdio",
      fields: [
        {
          path: ["command"],
          fieldRef: "cmd",
          exists: true,
          valueType: "string",
        },
        { path: ["args"], fieldRef: "args", exists: true, valueType: "array" },
      ],
    };
    function Fixture() {
      const [draft, setDraft] = useState(
        createMcpDraft({ name: "Docs", transport: "stdio" }),
      );
      return (
        <LibraryMcpForm
          value={draft}
          onChange={setDraft}
          initial={local}
          onSave={vi.fn()}
          onSaveEdits={save}
        />
      );
    }
    render(<Fixture />);
    fireEvent.change(screen.getByLabelText("参数"), {
      target: { value: "new-arg" },
    });
    await userEvent.click(screen.getByRole("combobox", { name: "args的操作" }));
    await userEvent.click(screen.getByRole("option", { name: "保留" }));
    await userEvent.click(screen.getByRole("combobox", { name: "args的操作" }));
    await userEvent.click(screen.getByRole("option", { name: "替换" }));
    expect(screen.getByLabelText("参数")).toHaveValue("new-arg");
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).toHaveBeenCalledWith(
      [{ path: ["args"], fieldRef: "args", action: "set", value: ["new-arg"] }],
      "draft",
    );
  });
  it("freezes an automatically generated identifier at the first submission attempt", async () => {
    const save = vi.fn();
    function Fixture() {
      const [draft, setDraft] = useState(createMcpDraft({ name: "Docs" }));
      return <LibraryMcpForm value={draft} onChange={setDraft} onSave={save} />;
    }
    render(<Fixture />);
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "First name" },
    });
    expect(screen.getByLabelText("配置标识")).toHaveValue("first-name");
    await userEvent.click(screen.getByRole("button", { name: "保存到资源库" }));
    expect(save).not.toHaveBeenCalled();
    fireEvent.change(screen.getByLabelText("名称"), {
      target: { value: "Second name" },
    });
    expect(screen.getByLabelText("配置标识")).toHaveValue("first-name");
  });
  it("names an unavailable target reason without exposing a backend key and keeps it keyboard reachable", async () => {
    i18n.addResourceBundle(
      "zh",
      "translation",
      { library: { nativeCliRequired: "请先安装原生 CLI。" } },
      true,
      true,
    );
    render(
      <LibraryMcpForm
        value={createMcpDraft()}
        onChange={vi.fn()}
        onSave={vi.fn()}
        targets={[
          {
            toolId: "codex",
            path: "",
            targetGroupId: "c",
            available: false,
            discoveryStatus: "missing",
            status: "unsupported",
            disabledReason: "library.nativeCliRequired",
          },
        ]}
        onToolsChange={vi.fn()}
      />,
    );
    const target = screen.getByRole("button", { name: "Codex" });
    expect(target).toHaveAttribute("title", "请先安装原生 CLI。");
    expect(target).toHaveAccessibleDescription("请先安装原生 CLI。");
    await act(async () => {
      target.focus();
    });
    expect(target).toHaveFocus();
    expect(target).toHaveAttribute("aria-disabled", "true");
  });
});

describe("lossless MCP configuration conversion", () => {
  it("accepts standard and tool-specific wrappers without dropping unknown fields", () => {
    for (const wrapper of ["mcpServers", "servers"]) {
      const drafts = mcpDraftsFromJson(
        JSON.stringify({
          [wrapper]: {
            first: { command: "server", timeout: 0 },
            second: {
              type: "http",
              url: "https://example.org/mcp",
              headers: { Authorization: "real-private-value" },
            },
          },
        }),
      );
      expect(drafts.map((draft) => draft.name)).toEqual(["first", "second"]);
      expect(mcpDraftToEdits(drafts[0])).toContainEqual({
        path: ["timeout"],
        action: "set",
        value: 0,
      });
    }
    const [local, remote] = mcpDraftsFromJson(
      JSON.stringify({
        mcp: {
          local: {
            type: "local",
            command: ["node", "", "/a b.js"],
            environment: { EMPTY: "" },
            enabled: false,
          },
          remote: {
            type: "remote",
            url: "https://example.org/mcp",
            headers: { Authorization: "real-value" },
          },
        },
      }),
    );
    expect(mcpDraftToEdits(local)).toContainEqual({
      path: ["args"],
      action: "set",
      value: ["", "/a b.js"],
    });
    expect(mcpDraftToEdits(local)).toContainEqual({
      path: ["enabled"],
      action: "set",
      value: false,
    });
    expect(remote.transport).toBe("http");
  });
  it.each([
    [
      '{"mcpServers":{"a":{"command":"one"},"a":{"command":"two"}}}',
      "duplicateJsonKey",
    ],
    [
      '{"type":"stdio","command":"s","headers":{},"url":"https://example.org"}',
      "advancedRequired",
    ],
    [
      '{"command":"s","headers":{"Authorization":"a","authorization":"b"}}',
      "advancedRequired",
    ],
    [
      '{"type":"http","url":"https://example.org","headers":{"Authorization":"a","authorization":"b"}}',
      "duplicateHeader",
    ],
    ['{"command":"s","env":{"TOKEN":"a","TOKEN":"b"}}', "duplicateJsonKey"],
    [
      '{"mcpServers":{"a":{"command":"s"}},"servers":{"a":{"command":"s"}}}',
      "ambiguousJson",
    ],
    ['{"servers":{"a":{"command":"s"}},"inputs":[]}', "advancedRequired"],
    ['{"command":"s","args":[1]}', "advancedRequired"],
    ['{"name":12,"command":"s"}', "advancedRequired"],
    ['{"command":"s","env":{"TOKEN":"[REDACTED]"}}', "maskedValue"],
    ['{"command":"s","timeout":1e400}', "invalidJson"],
  ])(
    "refuses unsafe or lossy conversion without echoing the input %s",
    (input, error) => {
      expect(() => mcpDraftsFromJson(input)).toThrow(new RegExp(`^${error}$`));
    },
  );
  it("does not allow a zero-length executable or poison path", () => {
    expect(() =>
      mcpDraftToEdits(
        createMcpDraft({ name: "s", command: " ", argsText: "" }),
      ),
    ).toThrow("missingCommand");
    expect(() =>
      mcpDraftFromJson(
        '{"name":"s","command":"s","__proto__":{"secret":"private"}}',
      ),
    ).toThrow("invalidPair");
  });
  it("rejects integers whose exact source value would be lost in JavaScript", () => {
    expect(() =>
      mcpDraftFromJson('{"command":"server","timeout":9007199254740993}'),
    ).toThrow("advancedRequired");
  });
  it.each([
    "9007199254740993e0",
    "9007199254740993.0",
    "9.007199254740993e15",
    "-9007199254740993e0",
    "1.00000000000000000001",
    "1e-9999",
  ])("rejects precision loss hidden in numeric literal %s", (literal) => {
    expect(() =>
      mcpDraftFromJson(`{"command":"server","nonce":${literal}}`),
    ).toThrow("advancedRequired");
  });
  it.each(["0", "-0", "1.2300", "0.1", "1e-7", "1.5e3", "9007199254740991"])(
    "preserves representable numeric literal %s",
    (literal) => {
      expect(
        mcpDraftToEdits(
          mcpDraftFromJson(
            `{"name":"s","command":"server","nonce":${literal}}`,
          ),
        ),
      ).toContainEqual({
        path: ["nonce"],
        action: "set",
        value: Number(literal),
      });
    },
  );
});

describe("command parsing never executes or expands input", () => {
  it("retains POSIX quoted boundaries, empty argv, escapes and literal backslashes", () => {
    expect(
      parseMcpCommand(
        'node "a b.js" "" \'literal\\path\' escaped\\ space',
        "posix",
      ),
    ).toEqual({
      command: "node",
      args: ["a b.js", "", "literal\\path", "escaped space"],
    });
  });
  it("handles Windows executable paths, backslashes and quote rules", () => {
    expect(
      parseMcpCommand(
        '"C:\\Program Files\\Node\\node.exe" "C:\\a b\\mcp.js" ""',
        "windows",
      ),
    ).toEqual({
      command: "C:\\Program Files\\Node\\node.exe",
      args: ["C:\\a b\\mcp.js", ""],
    });
    expect(parseMcpCommand("node C:\\tools\\server.js", "windows")).toEqual({
      command: "node",
      args: ["C:\\tools\\server.js"],
    });
  });
  it("preserves doubled double quotes inside a Windows quoted argument", () => {
    expect(parseMcpCommand('server "a""b"', "windows")).toEqual({
      command: "server",
      args: ['a"b'],
    });
  });
  it("requires an explicit HTTP/SSE transport when URL-only JSON cannot prove it", () => {
    expect(() =>
      mcpDraftsFromJson('{"url":"https://example.org/events"}'),
    ).toThrow("missingTransport");
  });
  it.each([
    "server | other",
    "server > file",
    "server $(secret)",
    "server `secret`",
    "TOKEN=secret server",
    "bash -c server",
    "C:\\Windows\\System32\\cmd.exe /c server",
    "pwsh -Command server",
    "env server",
    "sudo server",
    "npm.cmd server",
  ])("rejects shell syntax and wrappers: %s", (command) => {
    expect(() => parseMcpCommand(command, "windows")).toThrow("shellCommand");
  });
  it("rejects unclosed quotes instead of guessing boundaries", () => {
    expect(() => parseMcpCommand('node "unfinished', "posix")).toThrow(
      "invalidCommand",
    );
  });
});

describe("stored-field deltas and stable identifiers", () => {
  it("preserves all unrelated fields by omission, including credentials and unknown fields", () => {
    expect(
      mcpEditDelta(existing, [
        { path: ["url"], action: "set", value: "https://example.org/new" },
      ]),
    ).toEqual([
      {
        path: ["url"],
        action: "set",
        value: "https://example.org/new",
        fieldRef: "url-ref",
      },
    ]);
    expect(
      mcpEditDelta(existing, [{ path: ["timeout"], action: "keep" }]),
    ).toEqual([]);
    expect(
      mcpEditDelta(existing, [
        { path: ["headers", "Authorization"], action: "clear" },
      ]),
    ).toEqual([
      {
        path: ["headers", "Authorization"],
        action: "clear",
        fieldRef: "secret-ref",
      },
    ]);
  });
  it("requires explicit transport confirmation and clears only incompatible stored leaves", () => {
    const edits = [
      { path: ["type"], action: "set" as const, value: "stdio" },
      { path: ["command"], action: "set" as const, value: "server" },
    ];
    expect(() => mcpEditDelta(existing, edits)).toThrow("confirmTransport");
    expect(mcpEditDelta(existing, edits, true)).toEqual([
      { ...edits[0], fieldRef: "type-ref" },
      edits[1],
      { path: ["url"], fieldRef: "url-ref", action: "clear" },
      {
        path: ["headers", "Authorization"],
        fieldRef: "secret-ref",
        action: "clear",
      },
    ]);
  });
  it("rejects parents, overlaps and over-budget updates", () => {
    expect(() =>
      mcpEditDelta(existing, [{ path: ["headers"], action: "set", value: {} }]),
    ).toThrow("overlappingEdits");
    expect(() =>
      mcpEditDelta(existing, [
        { path: ["timeout"], action: "set", value: 1 },
        { path: ["timeout", "nested"], action: "set", value: 1 },
      ]),
    ).toThrow("overlappingEdits");
    expect(() =>
      mcpEditDelta(
        existing,
        Array.from({ length: 65 }, (_, i) => ({
          path: [`field${i}`],
          action: "set",
          value: i,
        })),
      ),
    ).toThrow("tooManyEdits");
  });
  it("uses readable identifiers with deterministic safe collision suffixes", () => {
    expect(createMcpIdentifier("Chrome DevTools")).toBe("chrome-devtools");
    expect(createMcpIdentifier("中文")).toBe("mcp");
    expect(createMcpIdentifier("Docs", ["docs"], "a1b2")).toBe("docs-a1b2");
    expect(createMcpIdentifier("Docs", ["docs", "docs-2"])).toBe("docs-3");
  });
  it("treats empty stored env/header containers as leaves when confirming a transport change", () => {
    const local = {
      ...existing,
      transport: "stdio",
      fields: [
        {
          path: ["command"],
          fieldRef: "cmd",
          valueType: "string",
          exists: true,
        },
        {
          path: ["env"],
          fieldRef: "empty-env",
          valueType: "object",
          exists: true,
        },
      ],
    };
    expect(
      mcpEditDelta(
        local,
        [
          { path: ["type"], action: "set", value: "http" },
          { path: ["url"], action: "set", value: "https://example.org/mcp" },
        ],
        true,
      ),
    ).toContainEqual({ path: ["env"], fieldRef: "empty-env", action: "clear" });
  });
  it("does not clear a required field while retaining its transport", () => {
    expect(() =>
      mcpEditDelta(existing, [{ path: ["url"], action: "clear" }]),
    ).toThrow("invalidUrl");
  });
});

describe("MCP multi-import identities", () => {
  it("shows collision-free IDs before saving, and passes edited selected IDs without replacing existing resources", async () => {
    const save = vi.fn();
    render(
      <LibraryMcpForm
        value={createMcpDraft()}
        onChange={vi.fn()}
        onSave={vi.fn()}
        onImportMany={save}
        existingIdentifiers={["docs"]}
      />,
    );
    await userEvent.click(screen.getByRole("radio", { name: "粘贴配置" }));
    fireEvent.change(screen.getByLabelText("MCP 配置 JSON"), {
      target: {
        value:
          '{"servers":{"Docs":{"command":"first"},"中文":{"command":"second"}}}',
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "识别配置" }));
    expect(screen.getByLabelText("Docs 的配置标识")).toHaveValue("docs-2");
    expect(screen.getByLabelText("中文 的配置标识")).toHaveValue("mcp");
    await userEvent.click(screen.getByRole("checkbox", { name: "选择 Docs" }));
    fireEvent.change(screen.getByLabelText("Docs 的配置标识"), {
      target: { value: "docs" },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "保存所选到资源库" }),
    );
    expect(save).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toHaveTextContent("配置标识已存在或重复");
    fireEvent.change(screen.getByLabelText("Docs 的配置标识"), {
      target: { value: "my-docs" },
    });
    await userEvent.click(
      screen.getByRole("button", { name: "保存所选到资源库" }),
    );
    expect(save).toHaveBeenCalledWith(
      [expect.objectContaining({ name: "Docs", identifier: "my-docs" })],
      "draft",
    );
  });
});
