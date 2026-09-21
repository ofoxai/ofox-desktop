import { describe, expect, it } from "vitest";
import {
  filterOfoxModelsForWorkBuddy,
  toWorkBuddyModelSelection,
  type FetchedModel,
} from "@/lib/api/model-fetch";

function model(overrides: Partial<FetchedModel> = {}): FetchedModel {
  return {
    id: "openai/gpt-test",
    name: "GPT Test",
    ownedBy: "openai",
    pricingPrompt: "0.000001",
    supportedEndpoints: ["/v1/chat/completions"],
    supportedParameters: ["tools", "reasoning"],
    inputModalities: ["text", "image"],
    outputModalities: ["text"],
    ...overrides,
  };
}

describe("WorkBuddy model compatibility", () => {
  it("keeps only text-output chat models with tool calls", () => {
    const compatible = model();
    const result = filterOfoxModelsForWorkBuddy([
      compatible,
      model({ id: "no-tools", supportedParameters: [] }),
      model({ id: "responses-only", supportedEndpoints: ["/v1/responses"] }),
      model({ id: "image-only", outputModalities: ["image"] }),
    ]);

    expect(result).toEqual([compatible]);
  });

  it("maps catalog capabilities into the native WorkBuddy entry", () => {
    expect(toWorkBuddyModelSelection(model())).toEqual({
      id: "openai/gpt-test",
      name: "GPT Test",
      supportsToolCall: true,
      supportsImages: true,
      supportsReasoning: true,
    });
  });
});
