import { StrictMode, type ReactNode } from "react";
import { act, render, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { useSessionPreload } from "@/hooks/useSessionPreload";
import { sessionsApi } from "@/lib/api/sessions";

vi.mock("@/hooks/useOfoxApex", () => ({
  useOfoxApex: () => ({ apex: "ofox.io", isLoading: false }),
}));
afterEach(() => vi.unstubAllGlobals());

it("cancels idle registration across StrictMode and retains the first list for a late visit", async () => {
  const callbacks = new Map<number, IdleRequestCallback>();
  let sequence = 0;
  vi.stubGlobal(
    "requestIdleCallback",
    vi.fn((callback: IdleRequestCallback) => {
      callbacks.set(++sequence, callback);
      return sequence;
    }),
  );
  const cancel = vi.fn((id: number) => callbacks.delete(id));
  vi.stubGlobal("cancelIdleCallback", cancel);
  const list = vi.spyOn(sessionsApi, "list").mockResolvedValue([]);
  const client = new QueryClient();
  function Preload() {
    useSessionPreload(true);
    return null;
  }
  const wrapper = ({ children }: { children: ReactNode }) => (
    <StrictMode>
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    </StrictMode>
  );
  const view = render(<Preload />, { wrapper });
  expect(cancel).toHaveBeenCalledTimes(1);
  expect(callbacks.size).toBe(1);
  expect(list).not.toHaveBeenCalled();
  await act(async () => {
    callbacks.values().next().value!({
      didTimeout: false,
      timeRemaining: () => 5,
    });
  });
  await waitFor(() =>
    expect(client.getQueryData(["sessions", "ofox.io"])).toEqual([]),
  );
  expect(list).toHaveBeenCalledTimes(1);
  expect(
    client.getQueryCache().find({ queryKey: ["sessions", "ofox.io"] })?.gcTime,
  ).toBe(Infinity);
  view.unmount();
  expect(callbacks.size).toBe(0);
  client.clear();
});
