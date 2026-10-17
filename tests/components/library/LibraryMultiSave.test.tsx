import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { expect, it, vi } from "vitest";
import { libraryApi } from "@/lib/api/library";
import { LibrarySection } from "@/components/library/LibrarySection";
vi.mock("@/components/library/LibraryMcpForm", async (importOriginal) => {
  const original =
    await importOriginal<
      typeof import("@/components/library/LibraryMcpForm")
    >();
  return {
    ...original,
    LibraryMcpForm: ({
      onImportMany,
    }: {
      onImportMany: (drafts: unknown[], intent: string) => void;
    }) => (
      <button
        onClick={() =>
          onImportMany(
            [
              {
                identifier: "first",
                name: "First",
                transport: "stdio",
                command: "server",
                argsText: "",
                url: "",
                environment: [],
                headers: [],
              },
              {
                identifier: "second",
                name: "Second",
                transport: "stdio",
                command: "server",
                argsText: "",
                url: "",
                environment: [],
                headers: [],
              },
            ],
            "draft",
          )
        }
      >
        Submit two
      </button>
    ),
  };
});
it("reports and refreshes a partially saved JSON selection without replaying the successful item", async () => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  await i18n.changeLanguage("zh");
  vi.spyOn(libraryApi, "list").mockResolvedValue({
    epoch: 1,
    servers: [],
    operations: [],
  });
  vi.spyOn(libraryApi, "inventory").mockResolvedValue({
    ticketId: "ticket",
    kind: "mcp",
    expiresAtMs: Date.now() + 60000,
    entries: [],
    sources: [],
  });
  vi.spyOn(libraryApi, "discardInventory").mockResolvedValue(true);
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([]);
  const save = vi
    .spyOn(libraryApi, "saveMcp")
    .mockResolvedValueOnce(undefined)
    .mockRejectedValueOnce(new Error("library.revisionChanged"));
  render(
    <QueryClientProvider client={new QueryClient()}>
      <LibrarySection
        kind="mcp"
        registerRefresh={() => () => undefined}
        registerDiscover={() => () => undefined}
        onChanged={vi.fn()}
      />
    </QueryClientProvider>,
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "添加 MCP" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "Submit two" }));
  expect(await screen.findByText("First · 已保存到资源库")).toBeVisible();
  expect(screen.getByText("Second · 未完成")).toBeVisible();
  expect(libraryApi.list).toHaveBeenCalledTimes(2);
  expect(save).toHaveBeenCalledTimes(2);
});
