import { describe, expect, it } from "vitest";
import type { AssistantModel } from "@/lib/api/assistant";
import {
  currentThinking,
  groupModelsByVendor,
  modelVendor,
  thinkingChoices,
} from "@/utils/assistantModels";

const model = (id: string, extra: Partial<AssistantModel> = {}) =>
  ({ id, name: id, ...extra }) as AssistantModel;
const names: Record<string, string> = {
  openai: "OpenAI",
  anthropic: "Anthropic",
  google: "Google",
  deepseek: "DeepSeek",
  qwen: "Qwen",
  "z-ai": "Z.ai",
  minimax: "MiniMax",
  "x-ai": "xAI",
};
const label = (vendor: string) => names[vendor] ?? vendor;
const group = (models: AssistantModel[]) =>
  groupModelsByVendor(models, label, "en");

describe("modelVendor", () => {
  it("is the vendor the backend sent", () => {
    expect(modelVendor(model("a/b", { vendor: "openai" }))).toBe("openai");
  });

  it("falls back to the id's prefix, and to none without one", () => {
    expect(modelVendor(model("qwen/qwen3-max"))).toBe("qwen");
    expect(modelVendor(model("plain"))).toBe("");
  });
});

describe("groupModelsByVendor", () => {
  it("lists OpenAI, Anthropic, Google, DeepSeek and Qwen first, in that order, then the others alphabetically by name", () => {
    const groups = group([
      model("z-ai/glm-5"),
      model("qwen/qwen3-max"),
      model("x-ai/grok-5"),
      model("deepseek/deepseek-v4"),
      model("minimax/minimax-m3"),
      model("google/gemini-3"),
      model("anthropic/claude-sonnet-5"),
      model("openai/gpt-5"),
    ]);
    expect(groups.map((entry) => entry.vendor)).toEqual([
      "openai",
      "anthropic",
      "google",
      "deepseek",
      "qwen",
      "minimax",
      "x-ai",
      "z-ai",
    ]);
  });

  it("names a vendor by its translation and an unknown one by its raw prefix", () => {
    const groups = group([model("openai/gpt-5"), model("newlab/model-1")]);
    expect(groups.map((entry) => entry.label)).toEqual(["OpenAI", "newlab"]);
  });

  it("sorts the unknown vendors in among the others by name", () => {
    const groups = group([
      model("z-ai/a"),
      model("newlab/a"),
      model("minimax/a"),
    ]);
    expect(groups.map((entry) => entry.label)).toEqual([
      "MiniMax",
      "newlab",
      "Z.ai",
    ]);
  });

  it("puts models without a vendor last", () => {
    const groups = group([model("plain"), model("z-ai/a"), model("openai/a")]);
    expect(groups.map((entry) => entry.vendor)).toEqual(["openai", "z-ai", ""]);
  });

  it("lists each vendor's models newest first, those without a date last, ties in catalog order", () => {
    const [openai] = group([
      model("openai/old", { created: 100 }),
      model("openai/undated"),
      model("openai/new", { created: 300 }),
      model("openai/mid-a", { created: 200 }),
      model("openai/mid-b", { created: 200 }),
      model("openai/null-date", { created: null }),
    ]);
    expect(openai.models.map((entry) => entry.id)).toEqual([
      "openai/new",
      "openai/mid-a",
      "openai/mid-b",
      "openai/old",
      "openai/undated",
      "openai/null-date",
    ]);
  });

  it("keeps every model once", () => {
    const input = [
      model("openai/a"),
      model("anthropic/b"),
      model("plain"),
      model("openai/c"),
    ];
    const ids = group(input).flatMap((entry) => entry.models.map((m) => m.id));
    expect([...ids].sort()).toEqual(input.map((entry) => entry.id).sort());
  });
});

describe("thinkingChoices", () => {
  it("offers the default, then the levels the model lists in its order", () => {
    expect(
      thinkingChoices(
        model("a/b", { thinkingLevels: ["low", "medium", "high"] }),
      ),
    ).toEqual(["default", "low", "medium", "high"]);
  });

  it("offers off only where the model lists it", () => {
    expect(
      thinkingChoices(model("a/b", { thinkingLevels: ["off", "low"] })),
    ).toEqual(["default", "off", "low"]);
    expect(thinkingChoices(model("a/b", { thinkingLevels: ["low"] }))).toEqual([
      "default",
      "low",
    ]);
  });

  it("offers nothing for a model that does not reason or is not known", () => {
    expect(thinkingChoices(model("a/b"))).toEqual([]);
    expect(thinkingChoices(model("a/b", { thinkingLevels: [] }))).toEqual([]);
    expect(thinkingChoices(undefined)).toEqual([]);
  });
});

describe("currentThinking", () => {
  const reasoning = model("a/b", { thinkingLevels: ["low", "high"] });

  it("is the saved level when the model offers it", () => {
    expect(currentThinking(reasoning, "high")).toBe("high");
  });

  it("is the default for no level and for a level the model no longer offers", () => {
    expect(currentThinking(reasoning, null)).toBe("default");
    expect(currentThinking(reasoning, undefined)).toBe("default");
    expect(currentThinking(reasoning, "xhigh")).toBe("default");
  });
});
