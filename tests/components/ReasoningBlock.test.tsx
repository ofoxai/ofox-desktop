import { act, render, screen } from "@testing-library/react";
import i18n from "i18next";
import {
  afterEach,
  beforeAll,
  beforeEach,
  describe,
  expect,
  it,
  vi,
} from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import { ReasoningBlock } from "@/components/assistant/ReasoningBlock";
import type { AssistantThinking } from "@/components/assistant/useAssistant";

beforeAll(() => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
});
beforeEach(async () => {
  await i18n.changeLanguage("en");
});
afterEach(() => vi.useRealTimers());

const live = (
  doneMs = 0,
  since: number | null = Date.now(),
): AssistantThinking => ({
  runId: "run",
  doneMs,
  since,
});
const after = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

describe("a reply's reasoning block", () => {
  it("keeps reasoning secondary when a task card already presents progress", () => {
    render(
      <ReasoningBlock
        compact
        reasoning="Compare possible layouts"
        thinking={live()}
      />,
    );
    expect(
      screen.getByText("Reasoning details").closest("summary"),
    ).not.toBeNull();
    expect(screen.queryByText(/Thinking for/)).toBeNull();
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByText("Compare possible layouts")).not.toBeVisible();
  });

  describe("while the reply is thinking", () => {
    beforeEach(() =>
      vi.useFakeTimers({ toFake: ["Date", "setInterval", "clearInterval"] }),
    );

    it("counts the seconds from the start of thinking, once a second", () => {
      render(<ReasoningBlock thinking={live()} />);
      // Rounded and never below one, as the finished caption says it.
      expect(screen.getByText("Thinking for 1 second")).toBeVisible();
      after(1600);
      expect(screen.getByText("Thinking for 2 seconds")).toBeVisible();
      after(1400);
      expect(screen.getByText("Thinking for 3 seconds")).toBeVisible();
    });

    it("adds the span under way to those that are over", () => {
      render(<ReasoningBlock thinking={live(4000)} />);
      after(2000);
      expect(screen.getByText("Thinking for 6 seconds")).toBeVisible();
    });

    it("says once, not every second, that the reply is thinking", () => {
      render(<ReasoningBlock thinking={live()} />);
      after(3000);
      expect(screen.getAllByRole("status")).toHaveLength(1);
      expect(screen.getByRole("status")).toHaveTextContent("Thinking…");
    });

    it("keeps the reasoning behind the caption while it streams", () => {
      render(<ReasoningBlock thinking={live()} reasoning="Let me think" />);
      expect(
        screen.getByText("Thinking for 1 second").closest("summary"),
      ).not.toBeNull();
      expect(screen.getByText("Let me think")).toBeInTheDocument();
    });

    it("stops counting once the span is over, and keeps what it spent", () => {
      const { rerender } = render(<ReasoningBlock thinking={live()} />);
      after(2400);
      rerender(
        <ReasoningBlock
          thinking={{ runId: "run", doneMs: 2400, since: null }}
        />,
      );
      expect(screen.getByText("Thought for 2 seconds")).toBeVisible();
      after(5000);
      expect(screen.getByText("Thought for 2 seconds")).toBeVisible();
      expect(screen.queryByRole("status")).toBeNull();
    });
  });

  describe("after the reply has thought", () => {
    it.each([
      [1, "Thought for 1 second"],
      [400, "Thought for 1 second"],
      [1499, "Thought for 1 second"],
      [1500, "Thought for 2 seconds"],
      [8400, "Thought for 8 seconds"],
      [62_000, "Thought for 62 seconds"],
    ])("rounds %i ms to whole seconds, never below one", (ms, expected) => {
      render(<ReasoningBlock reasoningMs={ms} reasoning="Because" />);
      expect(screen.getByText(expected)).toBeVisible();
    });

    it("keeps the reasoning in a collapsed disclosure under the caption", () => {
      render(<ReasoningBlock reasoningMs={3000} reasoning="Because" />);
      const details = screen
        .getByText("Thought for 3 seconds")
        .closest("details");
      expect(details).not.toBeNull();
      expect(details).not.toHaveAttribute("open");
      expect(details).toHaveTextContent("Because");
    });

    it("says how long even when the reasoning text itself was not kept", () => {
      render(<ReasoningBlock reasoningMs={3000} />);
      expect(screen.getByText("Thought for 3 seconds")).toBeVisible();
      expect(document.querySelector("details")).toBeNull();
    });

    it("takes the time of a finished stream from its spans", () => {
      render(
        <ReasoningBlock
          thinking={{ runId: "run", doneMs: 4200, since: null }}
          reasoning="Because"
        />,
      );
      expect(screen.getByText("Thought for 4 seconds")).toBeVisible();
    });

    it("hides a zero-duration finished span with no reasoning text", () => {
      const { container } = render(
        <ReasoningBlock thinking={{ runId: "run", doneMs: 0, since: null }} />,
      );
      expect(container).toBeEmptyDOMElement();
    });

    it.each([1, 400, 1499])(
      "keeps a positive %i ms finished span as a compact one-second caption",
      (doneMs) => {
        render(
          <ReasoningBlock thinking={{ runId: "run", doneMs, since: null }} />,
        );
        expect(screen.getByText("Thought for 1 second")).toBeVisible();
        expect(
          screen
            .getByText("Thought for 1 second")
            .closest('[data-reasoning="caption"]'),
        ).not.toBeNull();
        expect(document.querySelector("details")).toBeNull();
      },
    );
  });

  describe("when there is nothing to say", () => {
    it("shows nothing when the provider hides its reasoning", () => {
      const { container } = render(<ReasoningBlock />);
      expect(container).toBeEmptyDOMElement();
    });

    it("shows nothing for a null duration and no text", () => {
      const { container } = render(
        <ReasoningBlock reasoning={null} reasoningMs={null} thinking={null} />,
      );
      expect(container).toBeEmptyDOMElement();
    });

    it("keeps a plain 'Reasoning' disclosure for a reply saved without a duration", () => {
      render(<ReasoningBlock reasoning="Old reasoning" />);
      expect(
        screen.getByText("Reasoning").closest("details"),
      ).toHaveTextContent("Old reasoning");
    });
  });

  it.each([
    ["zh", "已思考 3 秒", "思考中 1 秒"],
    ["ja", "3 秒間思考しました", "1 秒間思考中"],
  ])("is worded in %s", async (language, done, liveText) => {
    await i18n.changeLanguage(language);
    const { rerender } = render(<ReasoningBlock reasoningMs={3000} />);
    expect(screen.getByText(done)).toBeVisible();
    rerender(<ReasoningBlock thinking={live()} />);
    expect(screen.getByText(liveText)).toBeVisible();
  });
});
