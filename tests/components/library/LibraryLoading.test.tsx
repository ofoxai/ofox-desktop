import { act, render, screen, waitFor } from "@testing-library/react";
import {
  QueryClient,
  QueryClientProvider,
  focusManager,
} from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { libraryApi, type LibraryKind } from "@/lib/api/library";
import { settingsApi } from "@/lib/api/settings";
import type { Settings } from "@/types";
import { LibrarySection } from "@/components/library/LibrarySection";
import { LibraryMcpTable } from "@/components/library/LibraryMcpTable";
import { LibrarySkillsPage } from "@/components/library/LibrarySkillsPage";
import { InteractionScope } from "@/components/ui/interaction-scope";

beforeEach(async () => {
  const i18n = (await import("i18next")).default;
  i18n.addResourceBundle(
    "zh",
    "translation",
    (await import("@/i18n/locales/zh.json")).default,
    true,
    true,
  );
  await i18n.changeLanguage("zh");
});

afterEach(() => {
  vi.restoreAllMocks();
  focusManager.setFocused(undefined);
});

it.each<LibraryKind>(["instructions", "mcp", "skills"])(
  "%s waits for routing, reuses a warm read, and refreshes explicitly",
  async (kind) => {
    let settingsDone!: (value: Settings) => void;
    vi.spyOn(settingsApi, "get").mockImplementation(
      () =>
        new Promise((resolve) => {
          settingsDone = resolve;
        }),
    );
    const list = vi.spyOn(libraryApi, "list").mockResolvedValue({
      epoch: 1,
      instructions: [],
      servers: [],
      skills: [],
      targets: [],
      operations: [],
    });
    vi.spyOn(libraryApi, "inventory").mockImplementation(
      () => new Promise(() => {}),
    );
    let refresh!: () => Promise<unknown>;
    const props = {
      registerRefresh: (_kind: LibraryKind, fn: () => Promise<unknown>) => {
        refresh = fn;
        return () => undefined;
      },
      registerDiscover: () => () => undefined,
      onChanged: () => undefined,
    };
    const page =
      kind === "skills" ? (
        <LibrarySkillsPage {...props} />
      ) : kind === "mcp" ? (
        <LibraryMcpTable {...props} />
      ) : (
        <LibrarySection kind={kind} {...props} />
      );
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    });
    const tree = (active: boolean) => (
      <QueryClientProvider client={client}>
        <InteractionScope active={active}>{page}</InteractionScope>
      </QueryClientProvider>
    );
    const view = render(tree(true));
    expect(list).not.toHaveBeenCalled();
    expect(
      await screen.findByRole("status", { name: "正在读取资源库…" }),
    ).toBeVisible();
    act(() =>
      settingsDone({ codexConfigDir: "/sandbox/custom-codex" } as Settings),
    );
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    await waitFor(() =>
      expect(
        screen.queryByRole("status", { name: "正在读取资源库…" }),
      ).not.toBeInTheDocument(),
    );
    // Window focus and a short return visit must not re-run a cold native scan.
    await act(async () => {
      focusManager.setFocused(false);
      focusManager.setFocused(true);
    });
    view.rerender(tree(false));
    view.rerender(tree(true));
    expect(list).toHaveBeenCalledTimes(1);
    await act(async () => {
      await refresh();
    });
    expect(list).toHaveBeenCalledTimes(2);
    client.clear();
  },
);

function routingSubject(kind: LibraryKind, active: boolean) {
  let refresh!: () => Promise<unknown>;
  const props = {
    registerRefresh: (_kind: LibraryKind, fn: () => Promise<unknown>) => {
      refresh = fn;
      return () => undefined;
    },
    registerDiscover: () => () => undefined,
    onChanged: () => undefined,
  };
  const page =
    kind === "skills" ? (
      <LibrarySkillsPage {...props} />
    ) : kind === "mcp" ? (
      <LibraryMcpTable {...props} />
    ) : (
      <LibrarySection kind={kind} {...props} />
    );
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  const tree = (visible: boolean) => (
    <QueryClientProvider client={client}>
      <InteractionScope active={visible}>{page}</InteractionScope>
    </QueryClientProvider>
  );
  const view = render(tree(active));
  return {
    show: () => view.rerender(tree(true)),
    refresh: () => refresh(),
    client,
  };
}

it.each<LibraryKind>(["instructions", "mcp", "skills"])(
  "%s does not read a hidden page and starts when shown",
  async (kind) => {
    const settings = vi.spyOn(settingsApi, "get").mockResolvedValue({
      codexConfigDir: "/sandbox/custom-codex",
    } as Settings);
    const list = vi.spyOn(libraryApi, "list").mockResolvedValue({
      epoch: 1,
      instructions: [],
      servers: [],
      skills: [],
      targets: [],
      operations: [],
    });
    vi.spyOn(libraryApi, "inventory").mockImplementation(
      () => new Promise(() => {}),
    );
    const subject = routingSubject(kind, false);
    await waitFor(() => expect(settings).toHaveBeenCalledTimes(1));
    expect(list).not.toHaveBeenCalled();
    subject.show();
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    subject.client.clear();
  },
);

it.each<LibraryKind>(["instructions", "mcp", "skills"])(
  "%s preserves unreadable routing and retries it before listing folders",
  async (kind) => {
    vi.spyOn(settingsApi, "get")
      .mockRejectedValueOnce(new Error("fixture routing unavailable"))
      .mockResolvedValueOnce({
        codexConfigDir: "/sandbox/custom-codex",
      } as Settings);
    const list = vi.spyOn(libraryApi, "list").mockResolvedValue({
      epoch: 1,
      instructions: [],
      servers: [],
      skills: [],
      targets: [],
      operations: [],
    });
    vi.spyOn(libraryApi, "inventory").mockImplementation(
      () => new Promise(() => {}),
    );
    const subject = routingSubject(kind, true);
    expect(await screen.findByRole("alert")).toBeVisible();
    expect(list).not.toHaveBeenCalled();
    await act(async () => {
      await subject.refresh();
    });
    await waitFor(() => expect(list).toHaveBeenCalledTimes(1));
    subject.client.clear();
  },
);
