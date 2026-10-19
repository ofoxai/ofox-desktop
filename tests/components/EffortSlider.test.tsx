import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { toast } from "sonner";
import { EffortSlider } from "@/components/model-picker/EffortSlider";
import { queryClient } from "@/lib/query/queryClient";
import {
  modelSelectionApi,
  type ToolModelSnapshot,
} from "@/lib/api/modelSelection";
import * as modelFetch from "@/lib/api/model-fetch";
import * as apexApi from "@/lib/api/ofoxApex";

function model(id: string, parameters: string[]) {
  return {
    id,
    name: id,
    ownedBy: id.split("/")[0],
    pricingPrompt: null,
    supportedEndpoints: ["/v1/responses", "/v1/chat/completions"],
    supportedParameters: parameters,
    inputModalities: ["text"],
    outputModalities: ["text"],
  };
}

function snapshot(
  value: string | null = "high",
  modelId = "openai/gpt-6",
): ToolModelSnapshot {
  return {
    toolId: "codex",
    writeTarget: "codex",
    revision: "rev-1",
    binding: {
      status: "configured",
      message: null,
      missingFiles: [],
      modifiedFields: [],
      envOverrides: [],
    },
    managedSelection: { kind: "single", model: modelId },
    observedSelection: { model: modelId, source: "config" },
    scope: [],
    effort: {
      value,
      levels: ["low", "medium", "high", "xhigh"],
      defaultLevel: "medium",
    },
  };
}

beforeEach(() => {
  queryClient.clear();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  vi.spyOn(apexApi, "ofoxGetApexState").mockResolvedValue({
    apex: "ofox.ai",
    pinned: true,
    detected: true,
  });
  vi.spyOn(modelFetch, "fetchOfoxModels").mockResolvedValue([
    model("openai/gpt-6", ["tools", "reasoning"]),
    model("openai/gpt-plain", ["tools"]),
  ]);
  vi.spyOn(modelSelectionApi, "saveEffort").mockImplementation(
    async (_tool, effort) => snapshot(effort),
  );
  vi.spyOn(toast, "success").mockImplementation(() => "");
  vi.spyOn(toast, "error").mockImplementation(() => "");
});
afterEach(() => vi.restoreAllMocks());

it("lays the tool's levels after a default stop and names the current one", async () => {
  render(<EffortSlider toolId="codex" snapshot={snapshot()} />);
  const slider = await screen.findByRole("slider", { name: "推理强度" });
  expect(slider).toHaveAttribute("aria-valuetext", "高");
  expect(slider).toHaveAttribute("aria-valuemax", "4");
  expect(screen.getByText("默认（中）")).toBeVisible();
  expect(screen.getByText("很高")).toBeVisible();
});

it("saves the level it moves to against the snapshot's revision", async () => {
  render(<EffortSlider toolId="codex" snapshot={snapshot()} />);
  const slider = await screen.findByRole("slider", { name: "推理强度" });
  fireEvent.keyDown(slider, { key: "ArrowRight" });
  expect(slider).toHaveAttribute("aria-valuetext", "很高");
  await waitFor(() =>
    expect(modelSelectionApi.saveEffort).toHaveBeenCalledWith(
      "codex",
      "xhigh",
      "rev-1",
    ),
  );
});

it("leaves the effort to the tool when moved to default", async () => {
  render(<EffortSlider toolId="codex" snapshot={snapshot()} />);
  const slider = await screen.findByRole("slider", { name: "推理强度" });
  fireEvent.keyDown(slider, { key: "Home" });
  expect(slider).toHaveAttribute("aria-valuetext", "默认（中）");
  await waitFor(() =>
    expect(modelSelectionApi.saveEffort).toHaveBeenCalledWith(
      "codex",
      null,
      "rev-1",
    ),
  );
});

it("puts the slider back and says so when the save fails", async () => {
  vi.mocked(modelSelectionApi.saveEffort).mockRejectedValue(
    new Error("configuration changed"),
  );
  render(<EffortSlider toolId="codex" snapshot={snapshot()} />);
  const slider = await screen.findByRole("slider", { name: "推理强度" });
  fireEvent.keyDown(slider, { key: "ArrowLeft" });
  await waitFor(() => expect(toast.error).toHaveBeenCalled());
  expect(slider).toHaveAttribute("aria-valuetext", "高");
});

it("keeps a level set outside Ofox as a stop of its own", async () => {
  render(<EffortSlider toolId="codex" snapshot={snapshot("minimal")} />);
  const slider = await screen.findByRole("slider", { name: "推理强度" });
  expect(slider).toHaveAttribute("aria-valuetext", "最低");
  expect(slider).toHaveAttribute("aria-valuemax", "5");
});

it("stays away for a model the catalog doesn't mark as reasoning", async () => {
  render(
    <EffortSlider
      toolId="codex"
      snapshot={snapshot("high", "openai/gpt-plain")}
    />,
  );
  await waitFor(() => expect(modelFetch.fetchOfoxModels).toHaveBeenCalled());
  expect(screen.queryByRole("slider")).toBeNull();
});
