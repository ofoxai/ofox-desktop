import {
  act,
  fireEvent,
  render,
  renderHook,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "@/i18n";
import en from "@/i18n/locales/en.json";
import { SkillPicker } from "@/components/assistant/SkillPicker";
import { useAssistant } from "@/components/assistant/useAssistant";
import { AssistantPage } from "@/components/assistant/AssistantPage";
import { ShellProvider } from "@/components/shell/ShellContext";

const ipc = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", async (original) => ({
  ...(await original<typeof import("@tauri-apps/api/core")>()),
  invoke: ipc.invoke,
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async () => () => undefined),
}));
vi.mock("@tauri-apps/plugin-notification", () => ({
  isPermissionGranted: vi.fn().mockResolvedValue(true),
  sendNotification: vi.fn(),
}));
const session = {
  id: "skills-session",
  title: "Skill conversation",
  assistantId: "ofox",
  capabilities: [],
  modelId: "test",
  webSearch: false,
  draft: "",
  draftAttachmentIds: [],
  createdAt: 1,
  updatedAt: 1,
  activeRun: null,
  deletedAt: null,
};
const skills = [
  {
    name: "auto",
    description: "Automatic instructions",
    mode: "auto",
    reviewed: true,
    origin: "reviewed",
  },
  {
    name: "manual",
    description: "Manual instructions",
    mode: "manual",
    reviewed: false,
    origin: "unreviewed",
  },
  {
    name: "mine",
    description: "Own instructions",
    mode: "auto",
    reviewed: false,
    origin: "local",
  },
  {
    name: "created",
    description: "Created instructions",
    mode: "auto",
    reviewed: false,
    origin: "assistantCreated",
  },
];
beforeEach(async () => {
  Element.prototype.scrollIntoView = vi.fn();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  ipc.invoke.mockReset().mockImplementation(async (name, args) => {
    if (name === "assistant_bootstrap")
      return {
        owner: "a".repeat(64),
        authorityGeneration: 1,
        runtime: { available: true },
        models: [
          {
            id: "test",
            name: "Test model",
            contextWindow: 128000,
            maxOutputTokens: 16000,
            supportsImages: false,
            pricing: {},
            inputModalities: ["text"],
          },
        ],
        defaultModelId: "test",
        key: { status: "ready" },
      };
    if (name === "assistant_list_sessions")
      return {
        sessions: [session],
        unreadable: [],
        trashCount: 0,
        matches: {},
      };
    if (name === "assistant_get_session")
      return {
        session: { ...session, id: args.sessionId },
        messages: [],
        attachments: [],
        usage: {},
      };
    if (name === "assistant_get_preferences")
      return { instructions: "", autoTitle: false };
    if (name === "assistant_update_session")
      return { ...session, ...args.request };
    if (name === "assistant_list_skills") return skills;
    if (name === "assistant_send") return args.request;
    if (name === "assistant_local_changes") return [];
    return {};
  });
});
async function open() {
  render(
    <ShellProvider openSettings={() => undefined}>
      <AssistantPage enabled />
    </ShellProvider>,
  );
  await userEvent.click(
    await screen.findByRole("button", { name: /Skill conversation/ }),
  );
  const input = await screen.findByRole("textbox", { name: "Message" });
  await waitFor(() => expect(input).toBeEnabled());
  return input;
}
it("typing / at the start opens Auto and Manual with honest origin labels", async () => {
  const input = await open();
  await userEvent.type(input, "/");
  const list = await screen.findByRole("listbox", { name: "Choose a skill" });
  expect(within(list).getByText("auto")).toBeVisible();
  expect(within(list).getByText("manual")).toBeVisible();
  expect(within(list).getByText("Unreviewed")).toBeVisible();
  expect(within(list).getByText("Assistant created")).toBeVisible();
});
it("keeps a long description to two lines, with the full text on hover", async () => {
  const input = await open();
  await userEvent.type(input, "/");
  const list = await screen.findByRole("listbox", { name: "Choose a skill" });
  const description = within(list).getByText("Automatic instructions");
  expect(description).toHaveClass("line-clamp-2");
  expect(description).toHaveAttribute("title", "Automatic instructions");
});
it("chooses by keyboard and sends the name through Page hook and real API IPC", async () => {
  const input = await open();
  await userEvent.type(input, "/man");
  await screen.findByRole("option", { name: /manual/ });
  fireEvent.keyDown(input, { key: "ArrowDown" });
  fireEvent.keyDown(input, { key: "Enter" });
  expect(
    ipc.invoke.mock.calls.some(([name]) => name === "assistant_send"),
  ).toBe(false);
  expect(screen.getByText("Skill: manual")).toBeVisible();
  await userEvent.type(input, "Use my words");
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() =>
    expect(ipc.invoke).toHaveBeenCalledWith("assistant_send", {
      request: expect.objectContaining({
        text: "Use my words",
        skill: "manual",
        uiLanguage: "en",
      }),
    }),
  );
});
it("Enter sends a slash message when no skill matches", async () => {
  const input = await open();
  await userEvent.type(input, "/etc/hosts");
  await screen.findByText("No available skills");
  fireEvent.keyDown(input, { key: "Enter" });
  await waitFor(() =>
    expect(ipc.invoke).toHaveBeenCalledWith("assistant_send", {
      request: expect.objectContaining({ text: "/etc/hosts" }),
    }),
  );
  const sent = ipc.invoke.mock.calls.find(
    ([name]) => name === "assistant_send",
  );
  expect(sent?.[1].request.skill).toBeUndefined();
});
it("Backspace removes a chosen chip, while a middle slash never opens", async () => {
  const input = await open();
  await userEvent.type(input, "hello /man");
  expect(screen.queryByRole("listbox")).toBeNull();
  fireEvent.change(input, { target: { value: "/man" } });
  await userEvent.click(await screen.findByRole("option", { name: /manual/ }));
  fireEvent.keyDown(input, { key: "Backspace" });
  expect(screen.queryByText("Skill: manual")).toBeNull();
});

it.each(["readOnly", "disabled"] as const)(
  "keeps the chip on Backspace while %s and removes it after becoming writable",
  async (restriction) => {
    const onSkill = vi.fn();
    const props = {
      value: "",
      skill: "manual",
      onSkill,
      onTextChange: vi.fn(),
      scope: "keyboard-restriction",
      "aria-label": "Message",
    };
    const { rerender } = render(
      <SkillPicker {...props} {...{ [restriction]: true }} />,
    );
    const input = screen.getByRole("textbox", { name: "Message" });
    await userEvent.click(input);
    await userEvent.keyboard("{Backspace}");
    expect(onSkill).not.toHaveBeenCalled();
    expect(screen.getByText("Skill: manual")).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Remove skill manual" }),
    ).toBeDisabled();
    rerender(<SkillPicker {...props} />);
    await userEvent.click(input);
    await userEvent.keyboard("{Backspace}");
    expect(onSkill).toHaveBeenCalledExactlyOnceWith(undefined);
  },
);

it("Escape closes the list and IME confirmation never sends or chooses", async () => {
  const input = await open();
  await userEvent.type(input, "/man");
  await screen.findByRole("option", { name: /manual/ });
  fireEvent.keyDown(input, { key: "Enter", keyCode: 229 });
  expect(screen.queryByText("Skill: manual")).toBeNull();
  expect(screen.getByRole("listbox")).toBeVisible();
  fireEvent.keyDown(input, { key: "Escape" });
  expect(screen.queryByRole("listbox")).toBeNull();
  expect(
    ipc.invoke.mock.calls.some(
      ([name]) => name === "assistant_send" || name === "assistant_stop",
    ),
  ).toBe(false);
});
it("does not call a local or assistant-created skill unreviewed", async () => {
  const input = await open();
  await userEvent.type(input, "/");
  const mine = await screen.findByRole("option", { name: /mine/ });
  const made = screen.getByRole("option", { name: /created/ });
  expect(within(mine).queryByText("Unreviewed")).toBeNull();
  expect(within(made).queryByText("Unreviewed")).toBeNull();
  expect(within(made).getByText("Assistant created")).toBeVisible();
});
it("deletes text before removing the chip and keeps ordinary Shift+Enter", async () => {
  const input = await open();
  await userEvent.type(input, "/man");
  await userEvent.click(await screen.findByRole("option", { name: /manual/ }));
  await userEvent.type(input, "words");
  await userEvent.keyboard("{Backspace}");
  expect(screen.getByText("Skill: manual")).toBeVisible();
  expect(input).toHaveValue("word");
  fireEvent.keyDown(input, { key: "Enter", shiftKey: true });
  expect(
    ipc.invoke.mock.calls.some(([name]) => name === "assistant_send"),
  ).toBe(false);
});

async function openHook() {
  const hook = renderHook(() => useAssistant(true));
  await waitFor(() => expect(hook.result.current.bootstrap).not.toBeNull());
  act(() => hook.result.current.select(session.id));
  await waitFor(() =>
    expect(hook.result.current.detail?.session.id).toBe(session.id),
  );
  return hook;
}
it("keeps unsent selections per session and clears them across region authority", async () => {
  const hook = await openHook();
  act(() => hook.result.current.setSkill("manual"));
  act(() => hook.result.current.select("other"));
  expect(hook.result.current.skill).toBeUndefined();
  act(() => hook.result.current.select(session.id));
  expect(hook.result.current.skill).toBe("manual");
  const original = ipc.invoke.getMockImplementation()!;
  ipc.invoke.mockImplementation(async (name, args) => {
    const result = await original(name, args);
    return name === "assistant_bootstrap"
      ? { ...result, authorityGeneration: 2 }
      : result;
  });
  await act(async () => {
    await hook.result.current.loadBootstrap();
  });
  expect(hook.result.current.skill).toBeUndefined();
});
it.each(["unchanged", "newChoice", "clearChoice", "roundTrip"])(
  "failed admission restores its chip without overwriting a newer choice (%s)",
  async (newChoice) => {
    const hook = await openHook();
    let reject: (error: string) => void = () => undefined;
    const failure = new Promise((_, no) => {
      reject = no;
    });
    const original = ipc.invoke.getMockImplementation()!;
    ipc.invoke.mockImplementation((name, args) =>
      name === "assistant_send" ? failure : original(name, args),
    );
    act(() => {
      hook.result.current.setSkill("manual");
      hook.result.current.setDraft("Original draft");
    });
    let sent: Promise<boolean>;
    act(() => {
      sent = hook.result.current.send();
    });
    await waitFor(() =>
      expect(
        ipc.invoke.mock.calls.some(([name]) => name === "assistant_send"),
      ).toBe(true),
    );
    if (newChoice === "newChoice")
      act(() => hook.result.current.setSkill("mine"));
    if (newChoice === "clearChoice")
      act(() => hook.result.current.setSkill(undefined));
    if (newChoice === "roundTrip")
      act(() => {
        hook.result.current.select("other");
        hook.result.current.select(session.id);
      });
    await act(async () => {
      reject("assistant_skill_unavailable");
      await sent!;
    });
    expect(hook.result.current.skill).toBe(
      newChoice === "newChoice"
        ? "mine"
        : newChoice === "clearChoice"
          ? undefined
          : "manual",
    );
    expect(hook.result.current.draft).toBe("Original draft");
  },
);

it("inline edit keeps original words and explicitly clears a historical chip", async () => {
  const original = ipc.invoke.getMockImplementation()!;
  ipc.invoke.mockImplementation(async (name, args) =>
    name === "assistant_get_session"
      ? {
          session,
          attachments: [],
          usage: {},
          messages: [
            {
              id: "historical-user",
              role: "user",
              content: "Stored original words",
              skill: "manual",
              status: "completed",
              createdAt: 1,
              nativeEntryId: "native-user",
              runId: "historical-run",
            },
          ],
        }
      : original(name, args),
  );
  await open();
  expect(screen.getByText("Stored original words")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "Edit and resend" }),
  );
  await userEvent.click(
    screen.getByRole("button", { name: "Remove skill manual" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Resend" }));
  await waitFor(() =>
    expect(ipc.invoke).toHaveBeenCalledWith("assistant_send", {
      request: expect.objectContaining({
        editMessageId: "historical-user",
        text: "Stored original words",
        replaceSkill: true,
      }),
    }),
  );
  const call = ipc.invoke.mock.calls.find(
    ([name]) => name === "assistant_send",
  )!;
  expect(call[1].request).not.toHaveProperty("skill");
});

it("resets picker navigation and ignores a late catalog across scope changes", async () => {
  let finishOld: (value: typeof skills) => void = () => undefined;
  const oldList = new Promise<typeof skills>((resolve) => {
    finishOld = resolve;
  });
  const original = ipc.invoke.getMockImplementation()!;
  let lists = 0;
  ipc.invoke.mockImplementation((name, args) =>
    name === "assistant_list_skills"
      ? ++lists === 1
        ? oldList
        : Promise.resolve([skills[2]])
      : original(name, args),
  );
  const selected = vi.fn();
  const view = render(
    <SkillPicker
      scope="owner:1:old-session"
      value="/"
      onTextChange={() => undefined}
      onSkill={selected}
      aria-label="Picker message"
    />,
  );
  await waitFor(() => expect(lists).toBe(1));
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "ArrowDown" });
  view.rerender(
    <SkillPicker
      scope="owner:2:new-session"
      value="/"
      onTextChange={() => undefined}
      onSkill={selected}
      aria-label="Picker message"
    />,
  );
  await userEvent.click(screen.getByRole("textbox"));
  await userEvent.keyboard("{End}");
  await screen.findByRole("option", { name: /mine/ });
  await act(async () => {
    finishOld(skills);
    await oldList;
  });
  expect(screen.queryByRole("option", { name: /manual/ })).toBeNull();
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Escape" });
  expect(screen.queryByRole("listbox")).toBeNull();
  view.rerender(
    <SkillPicker
      scope="other-owner:3:third-session"
      value="/"
      onTextChange={() => undefined}
      onSkill={selected}
      aria-label="Picker message"
    />,
  );
  await userEvent.click(screen.getByRole("textbox"));
  await userEvent.keyboard("{End}");
  await screen.findByRole("option", { name: /mine/ });
  fireEvent.keyDown(screen.getByRole("textbox"), { key: "Enter" });
  expect(selected).toHaveBeenCalledExactlyOnceWith("mine");
});
