import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import ManageToolDialog from "@/components/console/ManageToolDialog";
import { manageToolApi } from "@/lib/api/manageTool";
import * as modelFetch from "@/lib/api/model-fetch";
import type { FetchedModel } from "@/lib/api/model-fetch";

function compatibleModel(id: string): FetchedModel {
  return {
    id,
    name: `Name ${id}`,
    ownedBy: id.split("/")[0],
    pricingPrompt: "0.000001",
    supportedEndpoints: ["/v1/chat/completions"],
    supportedParameters: ["tools"],
    inputModalities: ["text"],
    outputModalities: ["text"],
  };
}

describe("ManageToolDialog WorkBuddy multi-model management", () => {
  it("selects every compatible model and saves the complete set", async () => {
    const models = [
      compatibleModel("openai/model-a"),
      compatibleModel("anthropic/model-b"),
      compatibleModel("google/model-c"),
    ];
    vi.spyOn(manageToolApi, "getConfigFilePath").mockResolvedValue(
      "/Users/test/.workbuddy/models.json",
    );
    vi.spyOn(manageToolApi, "getWorkBuddyManagedModels").mockResolvedValue([
      "openai/model-a",
    ]);
    vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue(models);
    const save = vi
      .spyOn(manageToolApi, "setWorkBuddyManagedModels")
      .mockResolvedValue();

    render(
      <ManageToolDialog
        tool={{
          id: "workbuddy",
          abbr: "WB",
          label: "WorkBuddy",
          color: "#111111",
          version: "5.5.6",
        }}
        onOpenChange={vi.fn()}
      />,
    );

    expect(
      await screen.findByText("共 3 个兼容模型，共用一个 Ofox Key"),
    ).toBeVisible();
    expect(screen.getByText("已选择 1 个兼容模型")).toBeVisible();

    fireEvent.click(screen.getByRole("button", { name: "全选兼容" }));
    expect(screen.getByText("已选择 3 个兼容模型")).toBeVisible();
    fireEvent.click(screen.getByRole("button", { name: "保存" }));

    await waitFor(() => {
      expect(save).toHaveBeenCalledWith(
        models.map(modelFetch.toWorkBuddyModelSelection),
      );
    });
  });
});
