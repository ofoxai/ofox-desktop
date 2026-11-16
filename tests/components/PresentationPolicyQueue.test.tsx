import { act, renderHook } from "@testing-library/react";
import { expect, it } from "vitest";
import { useMessageQueue } from "@/components/assistant/useMessageQueue";

it("freezes image limits with each queued turn instead of borrowing later composer settings", () => {
  const hook = renderHook(() => useMessageQueue());
  const policy = { mode: "auto" as const, maxImages: 1, maxCostUsd: "0.25" };
  act(() => {
    hook.result.current.controls.enqueue(
      "session",
      "PPT",
      undefined,
      "cartesian",
      policy,
    );
  });
  policy.maxImages = 4;
  policy.maxCostUsd = "9";
  expect(
    hook.result.current.controls.get("session")?.messages[0]
      .presentationVisualPolicy,
  ).toEqual({ mode: "auto", maxImages: 1, maxCostUsd: "0.25" });
});

it("clones references on enqueue and claim, preserving the frozen target while editing queue text", () => {
  const hook = renderHook(() => useMessageQueue());
  const references = [
    { attachmentId: "deck", version: "b".repeat(64), page: 2 },
  ];
  act(() => {
    hook.result.current.controls.enqueue(
      "session",
      "Improve this",
      undefined,
      undefined,
      undefined,
      references,
    );
  });
  references[0].page = 7;
  const message = hook.result.current.controls.get("session")!.messages[0];
  expect(message.fileReferences?.[0].page).toBe(2);
  act(() => {
    hook.result.current.controls.edit(
      "session",
      message.id,
      "Improve this one page",
    );
    hook.result.current.controls.resume("session");
  });
  let ticket!: ReturnType<typeof hook.result.current.controls.claim>;
  act(() => {
    ticket = hook.result.current.controls.claim("session");
  });
  message.fileReferences![0].page = 9;
  expect(ticket?.message.fileReferences?.[0].page).toBe(2);
});
