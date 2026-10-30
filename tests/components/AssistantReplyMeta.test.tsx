import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";
import type { AssistantMessage, AssistantModel } from "@/lib/api/assistant";
import type { TurnVersions } from "@/utils/assistantVersions";

// The line under a reply (model, time, usage, versions) and the card a failed
// or stopped reply carries: the parts of a message that are not its words.

vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: vi.fn() },
}));
vi.mock("sonner", () => ({ toast: { error: vi.fn() } }));
vi.mock("@/components/sessions/SessionMarkdown", () => ({
  SessionMarkdown: (props: { content: string }) => <p>{props.content}</p>,
}));

beforeAll(() => {
  // cmdk scrolls the highlighted option into view; jsdom has no layout.
  Element.prototype.scrollIntoView = vi.fn();
});
beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

const model = (
  id: string,
  name: string,
  extra: Partial<AssistantModel> = {},
): AssistantModel => ({
  id,
  name,
  contextWindow: 128_000,
  maxOutputTokens: 16_000,
  supportsImages: false,
  inputModalities: ["text"],
  supportedEndpoints: ["chat/completions"],
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
  ...extra,
});
const CATALOG = [
  model("openai/gpt-5-mini", "OpenAI: GPT-5 mini", {
    shortName: "GPT-5 mini",
    created: 100,
  }),
  model("anthropic/claude-sonnet-5", "Claude Sonnet 5", { created: 50 }),
];

type Props = Partial<Parameters<typeof AssistantMessageView>[0]>;
function view(message: Partial<AssistantMessage> = {}, props: Props = {}) {
  return (
    <AssistantMessageView
      message={{
        id: "m",
        role: "assistant",
        content: "The answer",
        createdAt: 1_000,
        status: "completed",
        nativeEntryId: "entry",
        runId: "run",
        ...message,
      }}
      models={CATALOG}
      disabled={false}
      onEdit={async () => false}
      onRegenerate={() => undefined}
      {...props}
    />
  );
}

const AT = new Date(2026, 9, 29, 14, 32).getTime();
const clock = (ms: number) =>
  new Intl.DateTimeFormat("en", { hour: "2-digit", minute: "2-digit" }).format(
    ms,
  );

describe("the line under a reply", () => {
  it("names the model as the picker does, and the time the reply ended", () => {
    render(view({ modelId: "openai/gpt-5-mini", completedAt: AT }, {}));
    expect(screen.getByText("GPT-5 mini")).toBeVisible();
    const time = screen.getByText(clock(AT));
    expect(time.tagName).toBe("TIME");
    expect(time).toHaveAttribute("datetime", new Date(AT).toISOString());
    // The full name and date are one hover away.
    expect(screen.getByText("GPT-5 mini")).toHaveAttribute(
      "title",
      "OpenAI: GPT-5 mini",
    );
    expect(time).toHaveAttribute("title");
  });

  it("falls back to the full name, then to the id, for a model the picker has no short name for", () => {
    const rendered = render(view({ modelId: "anthropic/claude-sonnet-5" }));
    expect(screen.getByText("Claude Sonnet 5")).toBeVisible();
    rendered.rerender(view({ modelId: "retired/model-9" }));
    expect(screen.getByText("retired/model-9")).toBeVisible();
  });

  it("says no model for a reply that has none, and keeps to its time", () => {
    render(view({ modelId: null, completedAt: AT }));
    expect(screen.getByText(clock(AT))).toBeVisible();
    expect(screen.queryByText(/·/)).toBeNull();
  });

  it("takes the start for the time when no end was saved", () => {
    render(view({ createdAt: AT, completedAt: null, modelId: "x/y" }));
    expect(screen.getByText(clock(AT))).toBeVisible();
  });

  it("is one quiet line, readable against the page", () => {
    render(view({ modelId: "openai/gpt-5-mini", completedAt: AT }));
    const meta = screen.getByText("GPT-5 mini").parentElement!;
    expect(meta).toHaveClass("text-ink-ctl", "text-note");
    expect(meta.closest(".flex-wrap")).toBeNull();
  });

  it("lets the model name be cut short at a narrow width: every flex wrapper may shrink", () => {
    render(view({ modelId: "openai/gpt-5-mini", completedAt: AT }));
    const name = screen.getByText("GPT-5 mini");
    expect(name).toHaveClass("truncate", "min-w-0");
    for (
      let wrapper = name.parentElement;
      wrapper && wrapper.tagName !== "ARTICLE";
      wrapper = wrapper.parentElement
    )
      expect(wrapper).toHaveClass("min-w-0");
  });

  it("is shown with the newest reply's actions and on hover or focus for the others", () => {
    const rendered = render(
      view({ modelId: "openai/gpt-5-mini", completedAt: AT }, { latest: true }),
    );
    const row = screen.getByText("GPT-5 mini").closest("[class*=opacity]");
    expect(row).not.toHaveClass("opacity-0");
    rendered.rerender(
      view(
        { modelId: "openai/gpt-5-mini", completedAt: AT },
        { latest: false },
      ),
    );
    expect(
      screen.getByText("GPT-5 mini").closest("[class*=opacity]"),
    ).toHaveClass("opacity-0", "group-hover:opacity-100");
  });

  it("is left off a question and a reply still being written", () => {
    const rendered = render(
      view({ role: "user", modelId: "openai/gpt-5-mini", completedAt: AT }),
    );
    expect(screen.queryByText("GPT-5 mini")).toBeNull();
    rendered.rerender(
      view({ status: "streaming", modelId: "openai/gpt-5-mini" }),
    );
    expect(screen.queryByText("GPT-5 mini")).toBeNull();
  });
});

describe("a reply's usage", () => {
  const usage = {
    inputTokens: 3,
    outputTokens: 312,
    estimatedCostUsd: "0.0004",
  };

  it("opens from the info button with the tokens, the cost and how long it took", async () => {
    render(view({ usage, createdAt: 1_000, completedAt: 5_240 }));
    const info = screen.getByRole("button", { name: "Reply details" });
    expect(info).toHaveAttribute("aria-expanded", "false");
    await userEvent.click(info);
    const popover = await screen.findByRole("dialog", {
      name: "Reply details",
    });
    expect(popover).toHaveTextContent(
      "Input 3 · Output 312 tokens · Estimated cost <$0.001",
    );
    expect(popover).toHaveTextContent("Took 4.2 sec");
  });

  it("says the cost is unavailable when a request's cost is unknown", async () => {
    render(
      view({
        usage: { ...usage, usageCoverage: "incomplete" },
        completedAt: 2_000,
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Reply details" }),
    );
    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Cost unavailable",
    );
  });

  it("has nothing to open when there is neither usage nor duration", () => {
    render(view({ usage: null, completedAt: null }));
    expect(screen.queryByRole("button", { name: "Reply details" })).toBeNull();
  });

  it("keeps the actions in view while it is open, though the pointer left the reply", async () => {
    render(view({ usage, completedAt: 2_000 }));
    const toolbar = screen.getByRole("button", { name: "Copy" }).parentElement!;
    expect(toolbar).toHaveClass("opacity-0");
    await userEvent.click(
      screen.getByRole("button", { name: "Reply details" }),
    );
    await screen.findByRole("dialog");
    expect(toolbar).not.toHaveClass("opacity-0");
  });
});

describe("the version switcher", () => {
  const versions: TurnVersions = {
    group: "start",
    index: 2,
    count: 3,
    previousId: "u1",
    nextId: "u3",
  };
  const group = () => screen.getByRole("group", { name: "Versions" });
  const previous = () =>
    within(group()).getByRole("button", { name: "Previous version" });
  const next = () =>
    within(group()).getByRole("button", { name: "Next version" });

  it("shows where the turn stands, in a live region that announces a change politely", () => {
    render(view({}, { versions, onSwitchVersion: vi.fn() }));
    const position = within(group()).getByText("2/3");
    expect(position).toHaveAttribute("aria-live", "polite");
    expect(position).toHaveAttribute("aria-atomic", "true");
  });

  it("is not there for a turn with one version", () => {
    render(view());
    expect(screen.queryByRole("group", { name: "Versions" })).toBeNull();
  });

  it("asks for the neighbouring question to be shown", async () => {
    const onSwitchVersion = vi.fn();
    render(view({}, { versions, onSwitchVersion }));
    await userEvent.click(previous());
    expect(onSwitchVersion).toHaveBeenLastCalledWith("u1");
    await userEvent.click(next());
    expect(onSwitchVersion).toHaveBeenLastCalledWith("u3");
  });

  it("can't go past the ends, yet keeps the focus the press had", async () => {
    const onSwitchVersion = vi.fn();
    render(
      view(
        {},
        {
          versions: { ...versions, index: 1, previousId: undefined },
          onSwitchVersion,
        },
      ),
    );
    expect(previous()).toHaveAttribute("aria-disabled", "true");
    expect(previous()).not.toBeDisabled();
    await userEvent.click(previous());
    expect(onSwitchVersion).not.toHaveBeenCalled();
    act(() => previous().focus());
    await userEvent.keyboard("{Enter}");
    expect(onSwitchVersion).not.toHaveBeenCalled();
    expect(previous()).toHaveFocus();
  });

  it("is off while a reply runs or a switch is under way", async () => {
    const onSwitchVersion = vi.fn();
    render(view({}, { versions, onSwitchVersion, disabled: true }));
    expect(previous()).toHaveAttribute("aria-disabled", "true");
    expect(next()).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(next());
    expect(onSwitchVersion).not.toHaveBeenCalled();
  });

  it("stays in place, with its focus, when another version is shown", () => {
    const rendered = render(
      view({ id: "r2" }, { versions, onSwitchVersion: vi.fn() }),
    );
    act(() => next().focus());
    const button = next();
    rendered.rerender(
      view(
        { id: "r3", content: "Another reply" },
        {
          versions: { ...versions, index: 3, nextId: undefined },
          onSwitchVersion: vi.fn(),
        },
      ),
    );
    expect(within(group()).getByText("3/3")).toBeVisible();
    expect(next()).toBe(button);
    expect(next()).toHaveFocus();
  });

  it("carries a question's switcher when the question has no reply", () => {
    render(
      view(
        { role: "user", content: "Question" },
        { versions, onSwitchVersion: vi.fn() },
      ),
    );
    expect(group()).toBeVisible();
  });
});

describe("the card of a failed or stopped reply", () => {
  const failed = (over: Partial<AssistantMessage> = {}) => ({
    status: "error" as const,
    content: "",
    errorCode: "assistant_network_error",
    ...over,
  });
  const retry = () => screen.getByRole("button", { name: "Retry" });
  const anotherModel = () =>
    screen.getByRole("combobox", { name: "Retry with another model" });

  it("says why and offers a retry, as plain text that is not announced", () => {
    render(view(failed(), { onRetry: vi.fn() }));
    expect(screen.getByText(en.assistant.errors.networkError)).toBeVisible();
    expect(retry()).toBeVisible();
    expect(screen.getByText("Failed")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("retries the reply", async () => {
    const onRetry = vi.fn();
    render(view(failed(), { onRetry }));
    await userEvent.click(retry());
    expect(onRetry).toHaveBeenCalledOnce();
    expect(onRetry).toHaveBeenCalledWith("m", undefined);
  });

  it("is on a stopped reply too, whose words stay", () => {
    render(
      view({ status: "stopped", content: "Half of it" }, { onRetry: vi.fn() }),
    );
    expect(screen.getByText("Half of it")).toBeVisible();
    expect(screen.getByText("Stopped")).toBeVisible();
    expect(retry()).toBeVisible();
  });

  it("is not there without a retry to offer", () => {
    render(view(failed()));
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
    // Without it the reason is still given.
    expect(screen.getByText(en.assistant.errors.networkError)).toBeVisible();
  });

  it("replaces the regenerate button, which would do the same", () => {
    const rendered = render(
      view({ status: "stopped", content: "Half" }, { onRetry: vi.fn() }),
    );
    expect(screen.queryByRole("button", { name: "Regenerate" })).toBeNull();
    rendered.rerender(view({ status: "stopped", content: "Half" }));
    expect(screen.getByRole("button", { name: "Regenerate" })).toBeVisible();
  });

  it("warns first that a search may already be billed", () => {
    render(
      view(failed({ errorCode: "assistant_search_outcome_unknown" }), {
        onRetry: vi.fn(),
      }),
    );
    const note = screen.getByText(en.assistant.retryBilledNote);
    expect(note).toBeVisible();
    expect(
      note.compareDocumentPosition(retry()) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    // The note says what happened: the generic reason is not said twice.
    expect(
      screen.queryByText(en.assistant.errors.searchOutcomeUnknown),
    ).toBeNull();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("stays pending until the send is over, and takes no second press", async () => {
    let finish: (sent: boolean) => void = () => undefined;
    const onRetry = vi.fn(
      () => new Promise<boolean>((resolve) => (finish = resolve)),
    );
    render(view(failed(), { onRetry }));
    await userEvent.click(retry());
    expect(retry()).toHaveAttribute("aria-disabled", "true");
    expect(retry()).toHaveAttribute("aria-busy", "true");
    // The menu is not `disabled` (that would drop the focus), but offers
    // nothing to pick.
    expect(anotherModel()).not.toBeDisabled();
    await userEvent.click(anotherModel());
    for (const option of within(
      await screen.findByRole("listbox"),
    ).getAllByRole("option"))
      expect(option).toHaveAttribute("aria-disabled", "true");
    await userEvent.keyboard("{Escape}");
    await userEvent.click(retry());
    expect(onRetry).toHaveBeenCalledOnce();
    // The send was refused: the card is ready again, with the focus it had.
    await act(async () => finish(false));
    expect(retry()).not.toHaveAttribute("aria-disabled");
    expect(retry()).not.toHaveAttribute("aria-busy");
    expect(retry()).toHaveFocus();
    await userEvent.click(retry());
    expect(onRetry).toHaveBeenCalledTimes(2);
  });

  it("is off while the page can't send", async () => {
    const onRetry = vi.fn();
    render(view(failed(), { onRetry, disabled: true }));
    expect(retry()).toHaveAttribute("aria-disabled", "true");
    expect(anotherModel()).not.toBeDisabled();
    await userEvent.click(retry());
    await userEvent.click(anotherModel());
    await userEvent.click(
      within(await screen.findByRole("listbox")).getByRole("option", {
        name: /Claude Sonnet 5/,
      }),
    );
    expect(onRetry).not.toHaveBeenCalled();
  });

  it("keeps the focus on the model menu after a pick, while the send is pending and the page turns busy", async () => {
    let finish: (sent: boolean) => void = () => undefined;
    const onRetry = vi.fn(
      () => new Promise<boolean>((resolve) => (finish = resolve)),
    );
    const props = { onRetry, currentModelId: "openai/gpt-5-mini" };
    const rendered = render(view(failed(), props));
    await userEvent.click(anotherModel());
    await userEvent.click(
      within(await screen.findByRole("listbox")).getByRole("option", {
        name: /Claude Sonnet 5/,
      }),
    );
    expect(onRetry).toHaveBeenCalledWith("m", "anthropic/claude-sonnet-5");
    await waitFor(() => expect(anotherModel()).toHaveFocus());
    // The send starts: the page can't send for a while, as the real one does.
    rendered.rerender(view(failed(), { ...props, disabled: true }));
    expect(anotherModel()).toHaveFocus();
    await act(async () => finish(false));
    rendered.rerender(view(failed(), props));
    expect(anotherModel()).toHaveFocus();
  });

  it("retries with another model picked from the picker's grouped list", async () => {
    const onRetry = vi.fn();
    render(view(failed(), { onRetry, currentModelId: "openai/gpt-5-mini" }));
    await userEvent.click(anotherModel());
    const list = await screen.findByRole("listbox");
    expect(
      within(list)
        .getAllByRole("group")
        .map((item) => item.getAttribute("aria-labelledby"))
        .map((id) => document.getElementById(id ?? "")?.textContent),
    ).toEqual(["OpenAI", "Anthropic"]);
    await userEvent.click(
      within(list).getByRole("option", { name: /Claude Sonnet 5/ }),
    );
    expect(onRetry).toHaveBeenCalledWith("m", "anthropic/claude-sonnet-5");
  });

  it("does not count the conversation's own model as another one", async () => {
    const onRetry = vi.fn();
    render(view(failed(), { onRetry, currentModelId: "openai/gpt-5-mini" }));
    await userEvent.click(anotherModel());
    await userEvent.click(
      within(await screen.findByRole("listbox")).getByRole("option", {
        name: /GPT-5 mini/,
      }),
    );
    expect(onRetry).not.toHaveBeenCalled();
  });
});

it("offers each frozen request receipt under reply details without claiming an actual charge", async () => {
  const user = userEvent.setup();
  render(
    view({
      usage: {
        inputTokens: 10,
        estimatedCostUsd: "0.00001",
        billedCostUsd: null,
        requests: [
          {
            kind: "chat",
            modelId: "retired/model",
            apex: "ofox.io",
            status: "completed",
            requestId: "req_gateway_123",
            requestIdSource: "x-ofox-request-id",
            httpStatus: 200,
            usage: { input: 10, output: 5 },
            priceSnapshot: { cost: { input: 2, output: 3 } },
            billedCostUsd: null,
          },
        ],
      } as AssistantMessage["usage"],
    }),
  );
  await user.click(screen.getByRole("button", { name: "Reply details" }));
  await user.click(screen.getByRole("button", { name: "Request records" }));
  const dialog = screen.getByRole("dialog", { name: "Request records" });
  expect(within(dialog).getByText("req_gateway_123")).toBeVisible();
  expect(within(dialog).getByText(/x-ofox-request-id/)).toBeVisible();
  expect(within(dialog).getByText("retired/model")).toBeVisible();
  expect(within(dialog).getByText(/Actual charge is unknown/)).toBeVisible();
  expect(within(dialog).getByRole("button", { name: "Close" })).toBeVisible();
  await user.click(within(dialog).getByText("Usage and frozen prices"));
  expect(within(dialog).getByText(/"input": 2/)).toBeVisible();
  await user.click(within(dialog).getByRole("button", { name: "Close" }));
  expect(screen.getByRole("button", { name: "Reply details" })).toHaveFocus();
});
