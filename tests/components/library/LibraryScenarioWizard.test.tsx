import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import {
  libraryApi,
  type LibraryData,
  type LibraryKind,
  type LibraryBatchPlan,
} from "@/lib/api/library";
import { LibraryScenarioWizard } from "@/components/library/LibraryScenarioWizard";
import { LIBRARY_SCENARIOS } from "@/components/library/libraryScenarios";
import { skillsApi } from "@/lib/api/skills";

const reviews = vi.hoisted(() => ({
  kinds: [] as string[],
  notify: undefined as undefined | ((plan: LibraryBatchPlan) => Promise<void>),
}));
vi.mock("@/components/library/LibraryBatchReview", () => ({
  LibraryBatchReview: (props: {
    kind: string;
    onChanged: (plan: LibraryBatchPlan) => Promise<void>;
  }) => {
    reviews.kinds.push(props.kind);
    reviews.notify = props.onChanged;
    return <p>Review {props.kind}</p>;
  },
}));
let epoch: number;
let changed: ReturnType<typeof vi.fn>;
const recipeIds: Record<LibraryKind, string[]> = {
  instructions: ["coding"],
  mcp: ["context7"],
  skills: ["frontend-design", "webapp-testing"],
};
function data(kind: LibraryKind): LibraryData {
  const resources = recipeIds[kind].map((id) => ({
    id,
    name: id,
    revision: 2,
    content: "fixture",
    transport: "http",
    credentialState: "none",
    fields: [],
    targets: [],
    directory: id,
    sourceState: "verified",
  }));
  return {
    epoch,
    operations: [],
    [kind === "mcp" ? "servers" : kind]: resources,
    targets: [
      {
        toolId: "codex",
        path: "/sandbox",
        targetGroupId: "codex",
        status: "unassigned",
        discoveryStatus: "installed",
        available: true,
      },
    ],
  };
}
beforeEach(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  epoch = 5;
  changed = vi.fn().mockResolvedValue(undefined);
  reviews.kinds = [];
  reviews.notify = undefined;
  vi.spyOn(libraryApi, "list").mockImplementation(async (kind) => data(kind));
  vi.spyOn(libraryApi, "matchMcpRecipes").mockResolvedValue([
    {
      recipeId: "context7",
      exactResourceIds: ["context7"],
      nativeEntryIds: [],
      similarResourceIds: [],
      similarNativeEntryIds: [],
    },
  ]);
  vi.spyOn(libraryApi, "matchBuiltinRecipes").mockImplementation(
    async (_kind, recipes) =>
      recipes.map((recipe) => ({
        recipeId: recipe.recipeId,
        exactResourceIds: [recipe.recipeId],
        nativeEntryIds: [],
        similarResourceIds: [],
        similarNativeEntryIds: [],
      })),
  );
  vi.spyOn(libraryApi, "saveInstruction");
  vi.spyOn(libraryApi, "saveMcp");
  vi.spyOn(skillsApi, "installUnified");
  vi.spyOn(libraryApi, "runBatch");
});
async function prepareFrontend() {
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  await userEvent.click(screen.getByRole("button", { name: "前端开发" }));
  expect(
    screen.getByRole("checkbox", { name: "Playwright" }),
  ).not.toBeChecked();
  await userEvent.click(
    screen.getByRole("checkbox", { name: "frontend-design" }),
  );
  await userEvent.click(
    screen.getByRole("checkbox", { name: "webapp-testing" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "补充配置并收录" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "使用已收录的「coding」" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "使用已收录的「context7」" }),
  );
  await userEvent.click(
    await screen.findByRole("button", { name: "选择工具" }),
  );
  expect(await screen.findByRole("button", { name: "Codex" })).toHaveAttribute(
    "aria-pressed",
    "false",
  );
  expect(screen.getByRole("button", { name: "查看总览" })).toBeDisabled();
  await userEvent.click(screen.getByRole("button", { name: "Codex" }));
  await userEvent.click(screen.getByRole("button", { name: "查看总览" }));
}
function result(kind: LibraryKind, status: string): LibraryBatchPlan {
  return {
    operationId: `sandbox-${kind}`,
    kind,
    status,
    resources: [],
    groups: [],
  };
}

it("keeps optional resources and tools off, reuses only confirmed exact resources, and reviews types separately", async () => {
  render(<LibraryScenarioWizard onChanged={changed} />);
  await prepareFrontend();
  expect(reviews.kinds).toEqual([]);
  expect(libraryApi.saveInstruction).not.toHaveBeenCalled();
  expect(libraryApi.saveMcp).not.toHaveBeenCalled();
  expect(skillsApi.installUnified).not.toHaveBeenCalled();
  await userEvent.click(screen.getByRole("button", { name: "开始逐类预览" }));
  expect(await screen.findByText("Review instructions")).toBeVisible();
  expect(screen.getByRole("button", { name: "预览下一类" })).toBeDisabled();
  await act(() => reviews.notify!(result("instructions", "partial")));
  expect(screen.getByRole("button", { name: "预览下一类" })).toBeDisabled();
  await act(() => reviews.notify!(result("instructions", "applied")));
  await userEvent.click(screen.getByRole("button", { name: "预览下一类" }));
  expect(await screen.findByText("Review mcp")).toBeVisible();
  expect(libraryApi.runBatch).not.toHaveBeenCalled();
});

it("stops later types and never replays completed work when reopened", async () => {
  render(<LibraryScenarioWizard onChanged={changed} />);
  await prepareFrontend();
  await userEvent.click(screen.getByRole("button", { name: "开始逐类预览" }));
  await screen.findByText("Review instructions");
  await act(() => reviews.notify!(result("instructions", "applied")));
  await userEvent.click(
    screen.getByRole("button", { name: "停止后续步骤并关闭" }),
  );
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  expect(screen.getByRole("button", { name: "前端开发" })).toBeVisible();
  expect(screen.queryByText("Review mcp")).not.toBeInTheDocument();
  expect(libraryApi.runBatch).not.toHaveBeenCalled();
});

it("rejects a restore epoch change before previewing the next type", async () => {
  render(<LibraryScenarioWizard onChanged={changed} />);
  await prepareFrontend();
  await userEvent.click(screen.getByRole("button", { name: "开始逐类预览" }));
  await screen.findByText("Review instructions");
  await act(() => reviews.notify!(result("instructions", "applied")));
  epoch++;
  await userEvent.click(screen.getByRole("button", { name: "预览下一类" }));
  expect(await screen.findByRole("alert")).toBeVisible();
  expect(screen.queryByText("Review mcp")).not.toBeInTheDocument();
});

it("blocks collection when old backends cannot verify existing resources", async () => {
  vi.mocked(libraryApi.matchBuiltinRecipes).mockRejectedValue(
    new Error("unknown command"),
  );
  render(<LibraryScenarioWizard onChanged={changed} />);
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  await userEvent.click(screen.getByRole("button", { name: "前端开发" }));
  await userEvent.click(screen.getByRole("button", { name: "补充配置并收录" }));
  await waitFor(() => expect(screen.getByRole("alert")).toBeVisible());
  expect(libraryApi.saveInstruction).not.toHaveBeenCalled();
  expect(skillsApi.installUnified).not.toHaveBeenCalled();
});

it("contains only the three approved kits and starts no optional dependency by default", () => {
  expect(LIBRARY_SCENARIOS.map((item) => item.id)).toEqual([
    "frontend",
    "docs",
    "review",
  ]);
  expect(
    LIBRARY_SCENARIOS.flatMap((item) =>
      item.resources
        .filter((resource) => resource.optional)
        .map((resource) => resource.recipeId),
    ),
  ).toEqual(["playwright", "context7", "chinese", "github"]);
});

it("returns keyboard focus to the scenario entry when closed", async () => {
  render(<LibraryScenarioWizard onChanged={changed} />);
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "场景套餐" })).toHaveFocus(),
  );
});

it("verifies a saved instruction after a failed refresh without creating a second copy", async () => {
  let saved:
    | { id: string; name: string; content: string; revision: number }
    | undefined;
  let failRefresh = true;
  vi.mocked(libraryApi.matchBuiltinRecipes).mockResolvedValue([]);
  vi.mocked(libraryApi.saveInstruction).mockImplementation(async (value) => {
    saved = { ...value, revision: 1 };
    return saved;
  });
  vi.mocked(libraryApi.list).mockImplementation(async (kind) => {
    if (saved && failRefresh) {
      failRefresh = false;
      throw new Error("refreshFailed");
    }
    return { ...data(kind), instructions: saved ? [saved] : [] };
  });
  render(<LibraryScenarioWizard onChanged={changed} />);
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  await userEvent.click(screen.getByRole("button", { name: "前端开发" }));
  for (const name of ["Context7", "frontend-design", "webapp-testing"])
    await userEvent.click(screen.getByRole("checkbox", { name }));
  await userEvent.click(screen.getByRole("button", { name: "补充配置并收录" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "加入我的资源" }),
  );
  await screen.findByRole("alert");
  await userEvent.click(
    screen.getByRole("button", { name: "重新核验已保存项" }),
  );
  expect(await screen.findByRole("button", { name: "选择工具" })).toBeVisible();
  expect(libraryApi.saveInstruction).toHaveBeenCalledTimes(1);
});

it("freezes a successfully saved MCP and offers verification instead of discarding later form edits", async () => {
  let written = false;
  let failRefresh = true;
  vi.mocked(libraryApi.matchMcpRecipes).mockResolvedValue([]);
  vi.mocked(libraryApi.saveMcp).mockImplementation(async () => {
    written = true;
  });
  vi.mocked(libraryApi.list).mockImplementation(async (kind) => {
    if (written && failRefresh) {
      failRefresh = false;
      throw new Error("refreshFailed");
    }
    return {
      ...data(kind),
      servers: written
        ? [
            {
              id: "microsoft-learn",
              name: "microsoft-learn",
              revision: 1,
              transport: "http",
              credentialState: "none",
              fields: [],
              targets: [],
            },
          ]
        : [],
    };
  });
  render(<LibraryScenarioWizard onChanged={changed} />);
  await userEvent.click(screen.getByRole("button", { name: "场景套餐" }));
  await userEvent.click(screen.getByRole("button", { name: "查文档" }));
  await userEvent.click(screen.getByRole("checkbox", { name: "DeepWiki" }));
  await userEvent.click(screen.getByRole("button", { name: "补充配置并收录" }));
  await userEvent.click(
    await screen.findByRole("button", { name: "保存到我的资源" }),
  );
  await screen.findByRole("alert");
  expect(screen.queryAllByRole("textbox")).toHaveLength(0);
  await userEvent.click(
    screen.getByRole("button", { name: "重新核验已保存项" }),
  );
  expect(await screen.findByRole("button", { name: "选择工具" })).toBeVisible();
  expect(libraryApi.saveMcp).toHaveBeenCalledTimes(1);
});
