import { render } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, expect, it, vi } from "vitest";
import { DeepLinkImportDialog } from "@/components/DeepLinkImportDialog";

const mocks = vi.hoisted(() => ({
  callbacks: new Map<string, (event: { payload: unknown }) => unknown>(),
  error: vi.fn(),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(
    async (
      name: string,
      callback: (event: { payload: unknown }) => unknown,
    ) => {
      mocks.callbacks.set(name, callback);
      return () => mocks.callbacks.delete(name);
    },
  ),
}));
vi.mock("sonner", () => ({ toast: { error: mocks.error } }));
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
afterEach(() => vi.restoreAllMocks());

it("uses translated generic copy for malformed links without logging private payloads", () => {
  const errors = vi.spyOn(console, "error").mockImplementation(() => {});
  render(
    <QueryClientProvider client={new QueryClient()}>
      <DeepLinkImportDialog />
    </QueryClientProvider>,
  );
  mocks.callbacks.get("deeplink-error")?.({
    payload: { url: "private-fixture", error: "private-fixture" },
  });
  expect(JSON.stringify(mocks.error.mock.calls)).not.toContain(
    "private-fixture",
  );
  expect(JSON.stringify(errors.mock.calls)).not.toContain("private-fixture");
  expect(mocks.error).toHaveBeenCalledWith("deeplink.parseError");
});
