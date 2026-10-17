import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { expect, it, vi } from "vitest";
import * as clipboard from "@/lib/clipboard";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { libraryApi } from "@/lib/api/library";
import { LibrarySourceDetail } from "@/components/library/LibrarySourceDetail";
import { LibraryMcpCompare } from "@/components/library/LibraryMcpCompare";
it("clears open source evidence after an external source change without another user action", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.useFakeTimers();
  try {
    vi.spyOn(libraryApi, "inventorySource")
      .mockResolvedValueOnce({
        ticketId: "t",
        entryId: "e",
        sourceRef: "s",
        toolId: "codex",
        label: "nativeMcp",
        locationKind: "file",
        isSymlink: false,
        observedAtMs: 0,
        canReveal: false,
        matchedResourceIds: [],
        fields: [{ path: ["original-field"], valueType: "string" }],
      })
      .mockRejectedValue(new Error("library.inventoryChanged"));
    render(
      <LibrarySourceDetail
        ticketId="t"
        entryId="e"
        sourceRef="s"
        epoch={1}
        onBack={vi.fn()}
        onLocate={vi.fn()}
      />,
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("original-field")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(screen.queryByText("original-field")).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("来源待确认");
  } finally {
    vi.useRealTimers();
  }
});
it("clears a displayed comparison when native evidence is no longer valid", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.useFakeTimers();
  try {
    vi.spyOn(libraryApi, "compareMcp")
      .mockResolvedValueOnce({
        fields: [{ path: ["original-comparison"], state: "equal" }],
      })
      .mockRejectedValue(new Error("library.inventoryChanged"));
    render(
      <LibraryMcpCompare
        epoch={1}
        subjects={[
          {
            name: "Left",
            subject: { kind: "native", ticketId: "t", entryId: "a" },
          },
          {
            name: "Right",
            subject: { kind: "native", ticketId: "t", entryId: "b" },
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "开始比较" }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.getByText("original-comparison")).toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(5000);
    });
    expect(screen.queryByText("original-comparison")).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("来源待确认");
  } finally {
    vi.useRealTimers();
  }
});

it("revalidates and uses the shared native clipboard path for copying a source", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  const detail = {
    ticketId: "t",
    entryId: "e",
    sourceRef: "s",
    toolId: "codex",
    label: "nativeMcp",
    locationKind: "file" as const,
    logicalPath: "/fixture/config.toml",
    isSymlink: false,
    observedAtMs: 0,
    canReveal: true,
    matchedResourceIds: [],
    fields: [],
  };
  const source = vi
    .spyOn(libraryApi, "inventorySource")
    .mockResolvedValue(detail);
  const copied = vi.spyOn(clipboard, "copyText").mockResolvedValue(undefined);
  render(
    <LibrarySourceDetail
      ticketId="t"
      entryId="e"
      sourceRef="s"
      epoch={1}
      onBack={vi.fn()}
      onLocate={vi.fn()}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "复制路径" }));
  await waitFor(() =>
    expect(copied).toHaveBeenCalledWith("/fixture/config.toml"),
  );
  expect(source).toHaveBeenCalledTimes(2);
});

it("does not resurrect an older polling success after a newer source action fails", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.useFakeTimers();
  try {
    const detail = {
      ticketId: "t",
      entryId: "e",
      sourceRef: "s",
      toolId: "codex",
      label: "nativeMcp",
      locationKind: "file" as const,
      logicalPath: "/fixture/config.toml",
      isSymlink: false,
      observedAtMs: 0,
      canReveal: true,
      matchedResourceIds: [],
      fields: [{ path: ["stale-poll-field"], valueType: "string" }],
    };
    let finish!: (value: typeof detail) => void;
    vi.spyOn(libraryApi, "inventorySource")
      .mockResolvedValueOnce(detail)
      .mockImplementationOnce(
        () =>
          new Promise((resolve) => {
            finish = resolve;
          }),
      )
      .mockRejectedValue(new Error("library.inventoryChanged"));
    render(
      <LibrarySourceDetail
        ticketId="t"
        entryId="e"
        sourceRef="s"
        epoch={1}
        onBack={vi.fn()}
        onLocate={vi.fn()}
      />,
    );
    await act(async () => {
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(5000);
    });
    fireEvent.click(screen.getByRole("button", { name: "复制路径" }));
    await act(async () => {
      await Promise.resolve();
    });
    expect(screen.queryByText("stale-poll-field")).not.toBeInTheDocument();
    await act(async () => {
      finish(detail);
      await Promise.resolve();
    });
    expect(screen.queryByText("stale-poll-field")).not.toBeInTheDocument();
  } finally {
    vi.useRealTimers();
  }
});
it("does not overlap slow comparison revalidation on timer and window focus", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.useFakeTimers();
  try {
    let fail!: (value: unknown) => void;
    const compare = vi
      .spyOn(libraryApi, "compareMcp")
      .mockResolvedValueOnce({
        fields: [{ path: ["slow-field"], state: "equal" }],
      })
      .mockImplementationOnce(
        () =>
          new Promise((_, reject) => {
            fail = reject;
          }),
      );
    render(
      <LibraryMcpCompare
        epoch={1}
        subjects={[
          {
            name: "Left",
            subject: { kind: "native", ticketId: "t", entryId: "a" },
          },
          {
            name: "Right",
            subject: { kind: "native", ticketId: "t", entryId: "b" },
          },
        ]}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: "开始比较" }));
    await act(async () => {
      await Promise.resolve();
      await vi.advanceTimersByTimeAsync(5000);
    });
    await act(async () => {
      window.dispatchEvent(new Event("focus"));
      await vi.advanceTimersByTimeAsync(10000);
    });
    expect(compare).toHaveBeenCalledTimes(2);
    await act(async () => {
      fail(new Error("library.inventoryChanged"));
      await Promise.resolve();
    });
    expect(screen.queryByText("slow-field")).not.toBeInTheDocument();
  } finally {
    vi.useRealTimers();
  }
});
