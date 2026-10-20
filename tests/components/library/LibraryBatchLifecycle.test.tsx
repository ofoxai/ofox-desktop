import { StrictMode } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { libraryApi, type LibraryBatchPlan } from "@/lib/api/library";
import { LibraryBatchReview } from "@/components/library/LibraryBatchReview";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";
const plan: LibraryBatchPlan = {
  operationId: "preview",
  kind: "mcp",
  status: "previewed",
  resources: [{ resourceId: "resource", expectedRevision: 4 }],
  groups: [],
};
const props = {
  kind: "mcp" as const,
  resources: [
    { resourceId: "resource", expectedRevision: 4, addTools: ["codex"] },
  ],
  tools: [] as string[],
  action: "apply" as const,
  epoch: 8,
  onBusyChange: vi.fn(),
  onChanged: vi.fn().mockResolvedValue(undefined),
};
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_ADD_COPY.zh },
    true,
    true,
  );
  vi.spyOn(libraryApi, "cancelBatch").mockResolvedValue({
    ...plan,
    status: "cancelled",
  });
});

it("cancels an untouched preview on actual unmount without touching StrictMode replay", async () => {
  const prepare = vi.spyOn(libraryApi, "previewBatch").mockResolvedValue(plan);
  const view = render(
    <StrictMode>
      <LibraryBatchReview {...props} />
    </StrictMode>,
  );
  await screen.findByText("已预览，尚未应用");
  expect(prepare).toHaveBeenCalledOnce();
  expect(libraryApi.cancelBatch).not.toHaveBeenCalled();
  view.unmount();
  await waitFor(() =>
    expect(libraryApi.cancelBatch).toHaveBeenCalledWith("preview"),
  );
});

it("disposes a blocked preview that arrives after the dialog closes", async () => {
  let finish!: (value: LibraryBatchPlan) => void;
  vi.spyOn(libraryApi, "previewBatch").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(<LibraryBatchReview {...props} />);
  view.unmount();
  await act(async () => finish({ ...plan, status: "blocked" }));
  expect(libraryApi.cancelBatch).toHaveBeenCalledOnce();
});

it("preserves prepared, partial, applied, and explicitly cancelled journals", async () => {
  for (const status of ["prepared", "partial", "applied", "cancelled"]) {
    const view = render(
      <LibraryBatchReview {...props} initial={{ ...plan, status }} />,
    );
    view.unmount();
    await act(async () => {
      await Promise.resolve();
    });
  }
  expect(libraryApi.cancelBatch).not.toHaveBeenCalled();
});

it("does not cancel an execution whose result has not reached the dialog", async () => {
  let finish!: (value: LibraryBatchPlan) => void;
  vi.spyOn(libraryApi, "runBatch").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(<LibraryBatchReview {...props} initial={plan} />);
  await userEvent.click(screen.getByRole("button", { name: "确认应用" }));
  view.unmount();
  await act(async () => finish({ ...plan, status: "partial" }));
  expect(libraryApi.cancelBatch).not.toHaveBeenCalled();
});

it("shares one cancellation between the explicit stop and unmount cleanup", async () => {
  let finish!: (value: LibraryBatchPlan) => void;
  vi.mocked(libraryApi.cancelBatch).mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const view = render(<LibraryBatchReview {...props} initial={plan} />);
  await userEvent.click(screen.getByRole("button", { name: "停止剩余项" }));
  view.unmount();
  await act(async () => {
    await Promise.resolve();
  });
  expect(libraryApi.cancelBatch).toHaveBeenCalledOnce();
  await act(async () => finish({ ...plan, status: "cancelled" }));
  expect(libraryApi.cancelBatch).toHaveBeenCalledOnce();
});

it("obtains a fresh preview for remaining changes without replaying the old operation", async () => {
  const prepare = vi
    .spyOn(libraryApi, "previewBatch")
    .mockResolvedValue({ ...plan, operationId: "new-preview" });
  const execute = vi.spyOn(libraryApi, "runBatch");
  render(
    <LibraryBatchReview {...props} initial={{ ...plan, status: "partial" }} />,
  );
  await userEvent.click(screen.getByRole("button", { name: "重新预览剩余项" }));
  expect(prepare).toHaveBeenCalledWith("mcp", props.resources, [], "apply", 8);
  expect(execute).not.toHaveBeenCalled();
  expect(await screen.findByText("new-preview")).not.toBeVisible();
});

it("asks history-only results to return to selection rather than inventing replay input", () => {
  render(
    <LibraryBatchReview
      {...props}
      resources={[]}
      initial={{ ...plan, status: "partial" }}
    />,
  );
  expect(
    screen.queryByRole("button", { name: "重新预览剩余项" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText("请返回资源列表，重新选择剩余项并预览。"),
  ).toBeVisible();
});
