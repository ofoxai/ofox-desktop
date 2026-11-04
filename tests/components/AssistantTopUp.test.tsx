import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { RetryCard } from "@/components/assistant/RetryCard";
import { GenerationCards } from "@/components/assistant/GenerationCards";
import type {
  AssistantGeneration,
  AssistantMessage,
} from "@/lib/api/assistant";

const state = vi.hoisted(() => ({
  apex: "ofox.io" as "ofox.ai" | "ofox.io",
  apexLoading: false,
  role: "owner" as "owner" | "admin" | "member",
  openExternal: vi.fn(),
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: state.apex, isLoading: state.apexLoading }),
}));
vi.mock("@/hooks/useOfoxAuth", () => ({
  useOfoxAuth: () => ({
    status: { user: { role: state.role } },
    isLoading: false,
  }),
}));
vi.mock("@/lib/api/settings", () => ({
  settingsApi: { openExternal: state.openExternal },
}));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  state.apex = "ofox.io";
  state.apexLoading = false;
  state.role = "owner";
  state.openExternal.mockReset().mockResolvedValue(undefined);
});

const retryCard = (errorCode: string | null) =>
  render(
    <RetryCard
      errorCode={errorCode}
      models={[]}
      currentModelId="chat/a"
      disabled={false}
      onRetry={vi.fn()}
    />,
  );

describe("a reply that ran out of balance", () => {
  it("opens the wallet of the account's region for an owner", async () => {
    retryCard("assistant_insufficient_balance");
    await userEvent.click(screen.getByRole("button", { name: "Top up" }));
    expect(state.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.io/manage/wallet",
    );
    expect(screen.getByRole("button", { name: "Retry" })).toBeInTheDocument();
  });

  it("waits for the region before it offers the wallet", () => {
    state.apexLoading = true;
    retryCard("assistant_insufficient_balance");
    expect(screen.getByRole("button", { name: "Top up" })).toBeDisabled();
  });

  it("tells a member to ask an admin instead", () => {
    state.role = "member";
    retryCard("assistant_insufficient_balance");
    expect(screen.queryByRole("button", { name: "Top up" })).toBeNull();
    expect(
      screen.getByText("Ask your team admin to top up."),
    ).toBeInTheDocument();
  });

  it("offers no top-up for another failure", () => {
    retryCard("assistant_network_error");
    expect(screen.queryByRole("button", { name: "Top up" })).toBeNull();
  });
});

describe("a generation that ran out of credits", () => {
  const generation = (status: string, errorCode: string | null) =>
    ({
      toolCallId: "call-1",
      kind: "speech",
      model: "speech/a",
      params: { voice: "alloy", responseFormat: "mp3" },
      inputIds: [],
      outputs: [],
      cost: null,
      status,
      errorCode,
      prompt: "Hello",
    }) as unknown as AssistantGeneration;
  const cards = (item: AssistantGeneration) =>
    render(
      <GenerationCards
        message={
          {
            id: "m1",
            role: "assistant",
            content: "",
            createdAt: 0,
            generations: [item],
          } as unknown as AssistantMessage
        }
        sessionId="s1"
        owner="o"
        generation={1}
        models={[]}
        onSave={vi.fn()}
        onCancelVideo={vi.fn()}
        onResumeVideo={vi.fn()}
      />,
    );

  it("offers the top-up beside the failure", async () => {
    state.apex = "ofox.ai";
    cards(generation("error", "assistant_media_insufficient_credits"));
    await userEvent.click(screen.getByRole("button", { name: "Top up" }));
    expect(state.openExternal).toHaveBeenCalledWith(
      "https://app.ofox.ai/manage/wallet",
    );
  });

  it("offers none for a generation that worked", () => {
    cards(generation("completed", null));
    expect(screen.queryByRole("button", { name: "Top up" })).toBeNull();
  });
});
