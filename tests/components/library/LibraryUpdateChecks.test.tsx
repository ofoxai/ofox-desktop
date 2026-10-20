import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import {
  libraryApi,
  type SkillDefinition,
  type SkillUpdateCheck,
} from "@/lib/api/library";
import { LibraryUpdateChecks } from "@/components/library/LibraryUpdateChecks";
import { LIBRARY_ADD_COPY } from "@/components/library/libraryAddCopy";
const skill = (id: string): SkillDefinition => ({
  id,
  name: id,
  directory: id,
  revision: 2,
  artifactHash: "hash",
  sourceState: "ready",
  targets: [],
});
const result = (id: string): SkillUpdateCheck => ({
  resourceId: id,
  revision: 2,
  epoch: 3,
  state: "updateAvailable",
  sourceVerified: true,
  currentArtifactHash: "old",
  candidate: {
    ticketId: `ticket-${id}`,
    expiresAtMs: Date.now() + 60000,
    artifactHash: "new",
    commitSha: "a".repeat(40),
    name: id,
  },
});

beforeEach(() => {
  i18n.addResourceBundle(
    "zh",
    "translation",
    { library: LIBRARY_ADD_COPY.zh },
    true,
    true,
  );
  vi.spyOn(libraryApi, "discardSkillUpdate").mockResolvedValue(true);
  vi.spyOn(libraryApi, "confirmSkillUpdate").mockResolvedValue({
    state: "unchanged",
    resourceId: "none",
    revision: 2,
    artifactHash: "hash",
    nativeApplyRequired: false,
  });
});

it("checks explicitly and sequentially, skips local modifications, and releases every candidate", async () => {
  let finish!: (value: SkillUpdateCheck) => void;
  const check = vi
    .spyOn(libraryApi, "checkSkillUpdate")
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          finish = resolve;
        }),
    )
    .mockResolvedValueOnce({
      ...result("second"),
      state: "upToDate",
      candidate: undefined,
    });
  const open = vi.fn();
  render(
    <LibraryUpdateChecks
      skills={[skill("first"), skill("second")]}
      epoch={3}
      onOpen={open}
      onBusyChange={vi.fn()}
    />,
  );
  expect(check).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "检查所选技能" }));
  expect(check).toHaveBeenCalledTimes(1);
  await act(async () =>
    finish({ ...result("first"), targetConflicts: ["/fixture/modified"] }),
  );
  await screen.findByText("已跳过：工具中的副本有本地修改。");
  expect(check).toHaveBeenCalledTimes(2);
  expect(libraryApi.discardSkillUpdate).toHaveBeenCalledWith("ticket-first");
  expect(libraryApi.confirmSkillUpdate).not.toHaveBeenCalled();
  await userEvent.click(
    screen.getAllByRole("button", { name: "查看更新详情" })[0],
  );
  expect(open).toHaveBeenCalledWith(skill("first"));
});

it("stops unstarted checks and cleans a candidate that arrives after cancellation", async () => {
  let finish!: (value: SkillUpdateCheck) => void;
  const check = vi.spyOn(libraryApi, "checkSkillUpdate").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const busy = vi.fn();
  render(
    <LibraryUpdateChecks
      skills={[skill("first"), skill("second")]}
      epoch={3}
      onOpen={vi.fn()}
      onBusyChange={busy}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "检查所选技能" }));
  await userEvent.click(screen.getByRole("button", { name: "停止检查" }));
  expect(busy).toHaveBeenLastCalledWith(false);
  expect(screen.getByRole("status")).toHaveTextContent("已检查 0 / 2");
  await act(async () => finish(result("first")));
  await waitFor(() =>
    expect(libraryApi.discardSkillUpdate).toHaveBeenCalledWith("ticket-first"),
  );
  expect(check).toHaveBeenCalledTimes(1);
  expect(screen.queryByText("有更新可查看")).not.toBeInTheDocument();
});

it("releases late tickets and parent busy state after unmount", async () => {
  let finish!: (value: SkillUpdateCheck) => void;
  vi.spyOn(libraryApi, "checkSkillUpdate").mockImplementation(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const busy = vi.fn();
  const view = render(
    <LibraryUpdateChecks
      skills={[skill("first")]}
      epoch={3}
      onOpen={vi.fn()}
      onBusyChange={busy}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "检查所选技能" }));
  view.unmount();
  expect(busy).toHaveBeenLastCalledWith(false);
  await act(async () => finish(result("first")));
  await waitFor(() =>
    expect(libraryApi.discardSkillUpdate).toHaveBeenCalledWith("ticket-first"),
  );
});

it("continues after a per-skill read failure without exposing exception text", async () => {
  const check = vi
    .spyOn(libraryApi, "checkSkillUpdate")
    .mockRejectedValueOnce(new Error("private-sentinel"))
    .mockResolvedValueOnce({ ...result("second"), sourceVerified: false });
  render(
    <LibraryUpdateChecks
      skills={[skill("first"), skill("second")]}
      epoch={3}
      onOpen={vi.fn()}
      onBusyChange={vi.fn()}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "检查所选技能" }));
  await screen.findByText("已跳过：资源库源文件有本地修改或无法核验。");
  expect(check).toHaveBeenCalledTimes(2);
  expect(screen.queryByText("private-sentinel")).not.toBeInTheDocument();
  expect(libraryApi.confirmSkillUpdate).not.toHaveBeenCalled();
});
