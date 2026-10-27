import { useEffect, useState } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AccountAssistantPanel } from "@/components/assistant/AccountAssistantPanel";
const mocks = vi.hoisted(() => ({
  user: {
    id: "fixture-a",
    org_id: "org-one",
    email: "fixture@example.com",
  } as {
    id: string | null;
    org_id: string | null;
    email: string | null;
    name?: string | null;
  } | null,
  apex: "ofox.ai",
  mounted: vi.fn(),
}));
vi.mock("@/hooks/useOfoxAccount", () => ({
  useOfoxAccount: () => ({ user: mocks.user }),
}));
vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: mocks.apex }),
}));
vi.mock("@/components/assistant/AssistantPage", () => ({
  AssistantPage: () => {
    const [draft, setDraft] = useState("");
    useEffect(() => {
      mocks.mounted();
    }, []);
    return (
      <input
        aria-label="Private draft"
        value={draft}
        onChange={(event) => setDraft(event.target.value)}
      />
    );
  },
  AssistantLoginRequired: ({
    onLoginRequested,
  }: {
    onLoginRequested?: () => void;
  }) => <button onClick={onLoginRequested}>Sign in</button>,
}));
beforeEach(() => {
  mocks.user = {
    id: "fixture-a",
    org_id: "org-one",
    email: "fixture@example.com",
  };
  mocks.apex = "ofox.ai";
  mocks.mounted.mockClear();
});

it("requires verified sign-in even when the previous account is still cached", () => {
  const onLoginRequested = vi.fn();
  render(
    <AccountAssistantPanel
      enabled={false}
      onLoginRequested={onLoginRequested}
    />,
  );
  expect(mocks.mounted).not.toHaveBeenCalled();
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(onLoginRequested).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Sign in" }));
  expect(onLoginRequested).toHaveBeenCalledOnce();
});

it("allows a verified session while its account profile is still loading", () => {
  mocks.user = null;
  render(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toBeVisible();
  expect(mocks.mounted).toHaveBeenCalledOnce();
  expect(screen.queryByRole("button", { name: "Sign in" })).toBeNull();
});

it("preserves an unsaved draft when the initial signed-in profile arrives", () => {
  mocks.user = null;
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Unsaved draft before the profile loaded" },
  });
  mocks.user = {
    id: "fixture-a",
    org_id: "org-one",
    email: "fixture@example.com",
  };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Unsaved draft before the profile loaded",
  );
  expect(mocks.mounted).toHaveBeenCalledOnce();
  mocks.user = { ...mocks.user, org_id: "another-org" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(mocks.mounted).toHaveBeenCalledTimes(2);
});

it("retains known identity through a missing profile and clears a later different account", () => {
  const original = mocks.user;
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Draft during a profile refresh" },
  });
  mocks.user = null;
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Draft during a profile refresh",
  );
  mocks.user = original;
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Draft during a profile refresh",
  );
  mocks.user = null;
  view.rerender(<AccountAssistantPanel enabled />);
  mocks.user = { ...original!, id: "different-account" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(mocks.mounted).toHaveBeenCalledTimes(2);
});

it("learns a missing organization without discarding the current draft", () => {
  mocks.user = { ...mocks.user!, org_id: null };
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Draft before organization metadata arrived" },
  });
  mocks.user = { ...mocks.user, org_id: "org-one" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Draft before organization metadata arrived",
  );
  expect(mocks.mounted).toHaveBeenCalledOnce();
  mocks.user = { ...mocks.user, org_id: "org-two" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(mocks.mounted).toHaveBeenCalledTimes(2);
});

it("preserves an older cached user's draft when the same email gains a stable ID", () => {
  mocks.user = { id: null, email: "fixture@example.com", org_id: "org-one" };
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Unsaved draft from an older cached profile" },
  });
  mocks.user = { ...mocks.user, id: "fixture-a" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Unsaved draft from an older cached profile",
  );
  expect(mocks.mounted).toHaveBeenCalledOnce();
});

it("does not treat a display-name edit as an identity change", () => {
  mocks.user = { id: null, email: null, org_id: "org-one", name: "Old name" };
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Draft while display metadata changes" },
  });
  mocks.user = { ...mocks.user, name: "New name" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue(
    "Draft while display metadata changes",
  );
  expect(mocks.mounted).toHaveBeenCalledOnce();
});

it.each([
  [
    "a conflicting known ID",
    { id: "fixture-a", email: "fixture@example.com" },
    { id: "fixture-b", email: "fixture@example.com" },
  ],
  [
    "a conflicting known email",
    { id: "fixture-a", email: "fixture@example.com" },
    { id: "fixture-a", email: "different@example.com" },
  ],
  [
    "a new ID with a different email",
    { id: null, email: "fixture@example.com" },
    { id: "fixture-b", email: "different@example.com" },
  ],
  [
    "a new ID without its matching email",
    { id: null, email: "fixture@example.com" },
    { id: "fixture-b", email: null },
  ],
  [
    "a new email without its matching ID",
    { id: "fixture-a", email: null },
    { id: null, email: "different@example.com" },
  ],
] as const)("clears private state for %s", (_reason, initial, next) => {
  mocks.user = { ...initial, org_id: "org-one" };
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Private draft belonging to the first identity" },
  });
  mocks.user = { ...next, org_id: "org-one" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(mocks.mounted).toHaveBeenCalledTimes(2);
});

it("unmounts private state on sign-out and starts a fresh workspace after sign-in", () => {
  const view = render(<AccountAssistantPanel enabled />);
  fireEvent.change(screen.getByRole("textbox"), {
    target: { value: "Private draft" },
  });
  view.rerender(<AccountAssistantPanel enabled={false} />);
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(screen.getByRole("button", { name: "Sign in" })).toBeVisible();
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  expect(mocks.mounted).toHaveBeenCalledTimes(2);
});

it("drops the displayed assistant state when the account, organization or region changes", () => {
  const view = render(<AccountAssistantPanel enabled />);
  const type = () =>
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "Private old-context draft" },
    });
  type();
  mocks.user = { ...mocks.user!, org_id: "org-two" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  type();
  mocks.user = { ...mocks.user!, id: "fixture-b" };
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
  type();
  mocks.apex = "ofox.io";
  view.rerender(<AccountAssistantPanel enabled />);
  expect(screen.getByRole("textbox")).toHaveValue("");
});
