import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "i18next";
import { beforeAll, beforeEach, describe, expect, it, vi } from "vitest";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";
import { AssistantModelPicker } from "@/components/assistant/AssistantModelPicker";
import type { AssistantModel } from "@/lib/api/assistant";

const model = (
  id: string,
  name: string,
  extra: Partial<AssistantModel> = {},
): AssistantModel => ({
  id,
  name,
  contextWindow: 128_000,
  maxOutputTokens: 16_000,
  supportsImages: false,
  inputModalities: ["text"],
  supportedEndpoints: ["chat/completions"],
  inputPricePerMillion: null,
  outputPricePerMillion: null,
  pricing: {},
  cost: { input: null, output: null, cacheRead: null, cacheWrite: null },
  pricingKnown: false,
  ...extra,
});

const CATALOG: AssistantModel[] = [
  model("z-ai/glm-5", "Z.ai: GLM 5", { shortName: "GLM 5", created: 10 }),
  model("qwen/qwen3-max", "Qwen: Qwen3 Max", {
    shortName: "Qwen3 Max",
    created: 10,
  }),
  model("openai/gpt-5-mini", "OpenAI: GPT-5 mini", {
    shortName: "GPT-5 mini",
    created: 100,
    reasoning: true,
    inputPricePerMillion: "0.25",
    outputPricePerMillion: "2",
  }),
  model("anthropic/claude-sonnet-5", "Claude Sonnet 5", {
    created: 50,
    supportsImages: true,
    reasoning: true,
  }),
  model("openai/gpt-5.4", "OpenAI: GPT-5.4", {
    shortName: "GPT-5.4",
    created: 300,
    supportsImages: true,
  }),
  model("newlab/model-1", "NewLab Model 1", { created: 5 }),
  model("google/gemini-3", "Google: Gemini 3", {
    shortName: "Gemini 3",
    created: 70,
  }),
  model("deepseek/deepseek-v4", "DeepSeek V4", { created: 20 }),
];

beforeAll(() => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle("ja", "translation", ja, true, true);
  // cmdk scrolls the highlighted option into view; jsdom has no layout.
  Element.prototype.scrollIntoView = vi.fn();
});
beforeEach(async () => {
  await i18n.changeLanguage("en");
});

function picker(
  props: Partial<Parameters<typeof AssistantModelPicker>[0]> = {},
) {
  return (
    <AssistantModelPicker
      models={CATALOG}
      value="openai/gpt-5.4"
      onChange={vi.fn()}
      partial={false}
      disabled={false}
      placeholder="Choose model"
      {...props}
    />
  );
}
const trigger = () => screen.getByRole("combobox", { name: "Choose model" });
async function open() {
  await userEvent.click(trigger());
  return screen.getByRole("listbox");
}

/** The vendor headings of the open list, in the order they are drawn. */
const headings = (listbox: HTMLElement) =>
  within(listbox)
    .getAllByRole("group")
    .map((group) => group.getAttribute("aria-labelledby"))
    .map((id) => document.getElementById(id ?? "")?.textContent);
const groupOptions = (listbox: HTMLElement, heading: string) =>
  within(within(listbox).getByRole("group", { name: heading }))
    .getAllByRole("option")
    .map((option) => option.textContent);

describe("the assistant's model picker", () => {
  it("groups the models by vendor: the fixed vendors first, the others by name", async () => {
    render(picker());
    expect(headings(await open())).toEqual([
      "OpenAI",
      "Anthropic",
      "Google",
      "DeepSeek",
      "Qwen",
      "newlab",
      "Z.ai",
    ]);
  });

  it("lists a vendor's models newest first, named without the vendor's own prefix", async () => {
    render(picker());
    const listbox = await open();
    // The note is part of the option's text: only the names are compared.
    const names = within(within(listbox).getByRole("group", { name: "OpenAI" }))
      .getAllByRole("option")
      .map((option) => option.textContent);
    expect(names[0]).toMatch(/^GPT-5\.4/);
    expect(names[1]).toMatch(/^GPT-5 mini/);
    expect(names).toHaveLength(2);
    expect(groupOptions(listbox, "Anthropic")[0]).toMatch(/^Claude Sonnet 5/);
  });

  it("names a vendor the translations lack by its raw prefix", async () => {
    render(picker());
    const listbox = await open();
    expect(
      within(listbox).getByRole("group", { name: "newlab" }),
    ).toBeVisible();
  });

  it.each([
    [
      "zh",
      [
        "OpenAI",
        "Anthropic",
        "Google",
        "深度求索 (DeepSeek)",
        "通义千问 (Qwen)",
      ],
    ],
    ["ja", ["OpenAI", "Anthropic", "Google", "DeepSeek", "Qwen"]],
  ])("names the vendors in %s", async (language, expected) => {
    await i18n.changeLanguage(language);
    render(picker({ placeholder: i18n.t("assistant.model") }));
    await userEvent.click(
      screen.getByRole("combobox", { name: i18n.t("assistant.model") }),
    );
    expect(headings(screen.getByRole("listbox")).slice(0, 5)).toEqual(expected);
  });

  it("filters across the vendors as the user types, matching a model's id and a vendor's name", async () => {
    render(picker());
    const listbox = await open();
    const field = screen.getByRole("combobox", { name: "Search models" });
    await userEvent.type(field, "claude");
    expect(headings(listbox)).toEqual(["Anthropic"]);
    await userEvent.clear(field);
    await userEvent.type(field, "deepseek");
    expect(headings(listbox)).toEqual(["DeepSeek"]);
    await userEvent.clear(field);
    await userEvent.type(field, "gpt");
    expect(headings(listbox)).toEqual(["OpenAI"]);
    expect(within(listbox).getAllByRole("option")).toHaveLength(2);
  });

  it("shows context, price and what the model can do under its name", async () => {
    render(picker());
    const listbox = await open();
    const mini = within(listbox).getByRole("option", { name: /GPT-5 mini/ });
    expect(mini).toHaveTextContent(
      "Context 128K tokens · Input $0.25 / output $2 per million tokens",
    );
    expect(within(mini).getByText("Reasoning")).toBeVisible();
    expect(within(mini).queryByText("Vision")).toBeNull();
    const sonnet = within(listbox).getByRole("option", {
      name: /Claude Sonnet 5/,
    });
    expect(within(sonnet).getByText("Reasoning")).toBeVisible();
    expect(within(sonnet).getByText("Vision")).toBeVisible();
    const gpt = within(listbox).getByRole("option", { name: /GPT-5\.4/ });
    expect(within(gpt).queryByText("Reasoning")).toBeNull();
    expect(within(gpt).getByText("Vision")).toBeVisible();
  });

  it("words the badges in the interface language", async () => {
    await i18n.changeLanguage("zh");
    render(picker({ placeholder: i18n.t("assistant.model") }));
    await userEvent.click(
      screen.getByRole("combobox", { name: i18n.t("assistant.model") }),
    );
    const sonnet = screen.getByRole("option", { name: /Claude Sonnet 5/ });
    expect(within(sonnet).getByText("推理")).toBeVisible();
    expect(within(sonnet).getByText("识图")).toBeVisible();
  });

  it("names the chosen model in full on its trigger", () => {
    render(picker());
    expect(trigger()).toHaveTextContent("OpenAI: GPT-5.4");
  });

  it("picks a model", async () => {
    const onChange = vi.fn();
    render(picker({ onChange }));
    await open();
    await userEvent.click(screen.getByRole("option", { name: /Gemini 3/ }));
    expect(onChange).toHaveBeenCalledWith("google/gemini-3");
  });

  describe("the kind switch", () => {
    it("offers chat models only for now, with their count", async () => {
      render(picker());
      await open();
      const kinds = screen.getByRole("radiogroup", { name: "Model type" });
      const radios = within(kinds).getAllByRole("radio");
      expect(radios).toHaveLength(1);
      expect(radios[0]).toHaveAccessibleName("Chat 8");
      expect(radios[0]).toBeChecked();
      expect(within(kinds).queryByRole("radio", { name: /Image/ })).toBeNull();
      expect(within(kinds).queryByRole("radio", { name: /Video/ })).toBeNull();
    });

    it("lists only chat models, and counts them alone", async () => {
      render(
        picker({
          models: [
            ...CATALOG,
            model("openai/gpt-image-2", "OpenAI: GPT Image 2", {
              kind: "image",
              created: 999,
            }),
            model("openai/sora-3", "OpenAI: Sora 3", { kind: "video" }),
          ],
        }),
      );
      const listbox = await open();
      expect(
        within(listbox).queryByRole("option", { name: /Image/ }),
      ).toBeNull();
      expect(
        within(listbox).queryByRole("option", { name: /Sora/ }),
      ).toBeNull();
      expect(
        within(
          screen.getByRole("radiogroup", { name: "Model type" }),
        ).getByRole("radio"),
      ).toHaveAccessibleName("Chat 8");
    });

    it("words the switch in the interface language", async () => {
      await i18n.changeLanguage("zh");
      render(picker({ placeholder: i18n.t("assistant.model") }));
      await userEvent.click(
        screen.getByRole("combobox", { name: i18n.t("assistant.model") }),
      );
      expect(
        within(screen.getByRole("radiogroup", { name: "模型类型" })).getByRole(
          "radio",
        ),
      ).toHaveAccessibleName("对话 8");
    });

    it("does not take the focus off the search field when it is clicked", async () => {
      render(picker());
      await open();
      const field = screen.getByRole("combobox", { name: "Search models" });
      await userEvent.click(screen.getByRole("radio", { name: /Chat/ }));
      expect(field).toHaveFocus();
    });
  });

  describe("when a catalog could not be read", () => {
    it("says some models are unavailable, in the menu", async () => {
      render(picker({ partial: true }));
      await open();
      expect(
        screen.getByText("Some models are temporarily unavailable"),
      ).toBeVisible();
    });

    it("says nothing when the list is complete", async () => {
      render(picker());
      await open();
      expect(screen.queryByText(/temporarily unavailable/)).toBeNull();
    });

    it("words the notice in zh and ja", () => {
      expect(i18n.getFixedT("zh")("assistant.modelsPartial")).toBe(
        "部分模型暂不可用",
      );
      expect(i18n.getFixedT("ja")("assistant.modelsPartial")).not.toBe("");
    });
  });

  it("shows the placeholder while the chosen model is not in the list", () => {
    render(picker({ value: "gone/model", placeholder: "gone/model" }));
    expect(
      screen.getByRole("combobox", { name: "Choose model" }),
    ).toHaveTextContent("gone/model");
  });
});
