import { describe, expect, it, vi } from "vitest";
import { createUsageRefreshCoordinator } from "@/lib/usageRefresh";

function deferred() {
  let resolve!: () => void;
  const promise = new Promise<void>((done) => (resolve = done));
  return { promise, resolve };
}

describe("usage refresh coordination", () => {
  it("merges automatic ticks and queues at most one manual follow-up", async () => {
    const first = deferred();
    const second = deferred();
    const run = vi
      .fn()
      .mockReturnValueOnce(first.promise)
      .mockReturnValueOnce(second.promise);
    const coordinator = createUsageRefreshCoordinator(run);
    const active = coordinator.request("manual");
    void coordinator.request("automatic");
    void coordinator.request("manual");
    void coordinator.request("manual");
    expect(run).toHaveBeenCalledTimes(1);
    first.resolve();
    await Promise.resolve();
    expect(run).toHaveBeenCalledTimes(2);
    second.resolve();
    await active;
    expect(run).toHaveBeenCalledTimes(2);
  });

  it("does not start queued work after its page lifetime ends", async () => {
    const first = deferred();
    const run = vi.fn().mockReturnValue(first.promise);
    const coordinator = createUsageRefreshCoordinator(run);
    const active = coordinator.request("manual");
    void coordinator.request("manual");
    coordinator.dispose();
    first.resolve();
    await active;
    expect(run).toHaveBeenCalledTimes(1);
  });
});

it("retains the same in-flight owner across hide and show", async () => {
  let release!: () => void;
  const run = vi
    .fn()
    .mockImplementationOnce(
      () =>
        new Promise<void>((resolve) => {
          release = resolve;
        }),
    )
    .mockResolvedValue(undefined);
  const owner = createUsageRefreshCoordinator(run);
  const first = owner.request("automatic");
  owner.setEnabled(false);
  await owner.request("automatic");
  owner.setEnabled(true);
  const second = owner.request("manual");
  expect(run).toHaveBeenCalledTimes(1);
  release();
  await Promise.all([first, second]);
  expect(run).toHaveBeenCalledTimes(2);
});
