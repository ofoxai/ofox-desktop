import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  checkToolUpdates,
  updateTools,
  useToolUpdates,
} from "@/hooks/useToolUpdates";
import { toolUpdatesApi, type ToolUpdateInfo } from "@/lib/api/toolUpdates";

vi.mock("@/lib/api/toolUpdates", () => ({
  toolUpdatesApi: {
    check: vi.fn(),
    update: vi.fn(),
  },
}));

const available: ToolUpdateInfo = {
  name: "codex",
  version: "0.160.0",
  latest_version: "0.161.0",
  error: null,
  installationKind: "cli",
  installationStatus: "installed",
  update_status: "available",
  update_source: "npm",
  update_supported: true,
  update_reason: null,
  executable_path: "/node/bin/codex",
};
const current: ToolUpdateInfo = {
  ...available,
  version: "0.161.0",
  update_status: "current",
};

beforeEach(async () => {
  vi.mocked(toolUpdatesApi.check).mockReset().mockResolvedValue([available]);
  vi.mocked(toolUpdatesApi.update).mockReset();
  await checkToolUpdates();
  vi.mocked(toolUpdatesApi.check).mockClear();
});

describe("useToolUpdates", () => {
  it("rechecks an external upgrade on focus even while the mount cache is fresh", async () => {
    const { result } = renderHook(() => useToolUpdates());
    expect(result.current.tools[0].update_status).toBe("available");
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();
    vi.mocked(toolUpdatesApi.check).mockResolvedValue([current]);

    act(() => window.dispatchEvent(new Event("focus")));

    await waitFor(() => expect(result.current.tools).toEqual([current]));
    expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1);
  });

  it("shares a single focus check across subscribers and concurrent focus events", async () => {
    let resolve!: (tools: ToolUpdateInfo[]) => void;
    vi.mocked(toolUpdatesApi.check).mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const first = renderHook(() => useToolUpdates());
    const second = renderHook(() => useToolUpdates());

    act(() => {
      window.dispatchEvent(new Event("focus"));
      window.dispatchEvent(new Event("focus"));
    });

    expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1);
    await act(async () => resolve([current]));
    expect(first.result.current.tools).toEqual([current]);
    expect(second.result.current.tools).toEqual([current]);
  });

  it("does not scan while disabled or after the subscriber unmounts", () => {
    const { rerender, unmount } = renderHook(
      ({ enabled }) => useToolUpdates(enabled),
      { initialProps: { enabled: false } },
    );
    act(() => window.dispatchEvent(new Event("focus")));
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();
    rerender({ enabled: true });
    unmount();
    act(() => window.dispatchEvent(new Event("focus")));
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();
  });

  it("waits for an in-app update to finish before scanning its installed version", async () => {
    let resolve!: () => void;
    vi.mocked(toolUpdatesApi.update).mockImplementation(async () => {
      await new Promise<void>((done) => {
        resolve = done;
      });
      return {
        status: "updated",
        before: available.version!,
        after: current.version!,
      };
    });
    const { result } = renderHook(() => useToolUpdates());
    let updating!: Promise<void>;
    act(() => {
      updating = updateTools(["codex"]);
    });
    await waitFor(() => expect(toolUpdatesApi.update).toHaveBeenCalledTimes(1));
    act(() => window.dispatchEvent(new Event("focus")));
    expect(toolUpdatesApi.check).not.toHaveBeenCalled();

    vi.mocked(toolUpdatesApi.check).mockResolvedValue([current]);
    await act(async () => {
      resolve();
      await updating;
    });
    expect(result.current.tools).toEqual([current]);
    expect(toolUpdatesApi.check).toHaveBeenCalledTimes(1);
  });
});
