import { createInstance } from "i18next";
import { describe, expect, it } from "vitest";
import en from "@/i18n/locales/en.json";
import zh from "@/i18n/locales/zh.json";
import ja from "@/i18n/locales/ja.json";
import { assistantMarkdown } from "@/utils/assistantMarkdown";
import type {
  AssistantMessage,
  AssistantSessionDetail,
} from "@/lib/api/assistant";

const file = {
  id: "file",
  name: "报告[1].docx",
  mimeType:
    "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
  sizeBytes: 1024,
  status: "partial" as const,
  reason: "assistant_attachment_partial",
  details: { format: "docx", textOnly: true },
};
const fixture = (): AssistantSessionDetail => ({
  session: {
    id: "session",
    title: "Notes [team]",
    titleSource: "user",
    assistantId: "ofox",
    capabilities: [],
    modelId: "test/model",
    webSearch: true,
    draft: "Continue later",
    draftAttachmentIds: [file.id],
    createdAt: 1,
    updatedAt: 2,
    deletedAt: null,
    pinnedAt: null,
    activeRun: null,
  },
  messages: [
    {
      id: "old",
      role: "user",
      content: "Superseded question",
      superseded: true,
      createdAt: 0,
    },
    {
      id: "internal",
      role: "tool",
      content: "assistant_request internal",
      createdAt: 0,
    } as unknown as AssistantMessage,
    {
      id: "user",
      role: "user",
      content: "Read this file.\n\n```ts\nconst value = 1;\n```",
      attachments: [file],
      createdAt: 1,
    },
    {
      id: "reply",
      role: "assistant",
      content: "A cited answer [1].",
      createdAt: 2,
      status: "error",
      errorCode: "assistant_search_outcome_unknown",
      sources: [
        {
          id: "1",
          title: "Docs [reference]",
          url: "https://example.com/docs?q=test",
          excerpt: "",
        },
        {
          id: "2",
          title: "Do not link",
          url: "javascript:alert(1)",
          excerpt: "",
        },
      ],
    },
  ],
  attachments: [file],
  usage: {
    inputTokens: 120,
    outputTokens: 40,
    estimatedCostUsd: "0.123456789123456789",
    searchCount: 2,
  },
});
async function render(detail: AssistantSessionDetail, language = "en") {
  const i18n = createInstance();
  await i18n.init({
    lng: language,
    resources: {
      en: { translation: en },
      zh: { translation: zh },
      ja: { translation: ja },
    },
    interpolation: { escapeValue: false },
  });
  return assistantMarkdown(detail, i18n.t, language);
}

describe("assistant Markdown export", () => {
  it("includes readable turns, code, safe sources, attachment scope and the unsent draft", async () => {
    const text = await render(fixture());
    expect(text).toContain("# Notes \\[team\\]");
    expect(text).toContain("```ts\nconst value = 1;\n```");
    expect(text).toContain(
      "[Docs \\[reference\\]](<https://example.com/docs?q=test>)",
    );
    expect(text).toContain("报告\\[1\\].docx");
    expect(text).toContain("Only text was read.");
    expect(text).toContain(
      "headers, footers, comments and hidden text were not",
    );
    expect(text).toContain("Continue later");
    expect(text).toContain("not embedded");
    expect(text).not.toMatch(
      /Superseded question|assistant_request|javascript:/,
    );
  });
  it("keeps the exact cost and localized uncertain-search reason without internal codes", async () => {
    const text = await render(fixture());
    expect(text).toContain("Estimated cost (USD): 0.123456789123456789");
    expect(text).toContain("Search charges (included in estimated cost): 2");
    expect(text).toContain("may have been charged");
    expect(text).not.toContain("assistant_search_outcome_unknown");
    const unknown = fixture();
    unknown.usage.usageCoverage = "incomplete";
    unknown.usage.searchCount = 0;
    expect(await render(unknown)).not.toContain("Search charges");
    expect(await render(unknown)).toContain("Cost unavailable");
    expect(await render(unknown)).not.toContain("0.123456789123456789");
  });
  it.each([
    ["zh", "你", "仅提取了文字"],
    ["ja", "あなた", "テキスト"],
  ])(
    "localizes the exported headings and file warnings (%s)",
    async (lang, role, warning) => {
      const text = await render(fixture(), lang);
      expect(text).toContain(`## ${role}`);
      expect(text).toContain(warning);
      expect(text).not.toMatch(/assistant\.(export|attachment|roles)/);
    },
  );
});

it("exports generated media metadata and files without private resource URLs", async () => {
  const detail = fixture();
  detail.messages.push({
    id: "media",
    role: "media",
    content: "",
    createdAt: 3,
    attachments: [
      {
        ...file,
        id: "image",
        name: "generated.png",
        mimeType: "image/png",
        status: "ready",
        reason: null,
        details: null,
      },
    ],
    media: {
      kind: "image",
      model: "image/test",
      params: { size: "1024x1024", quality: "low" },
      inputIds: [],
      outputs: ["image"],
      status: "completed",
      cost: "0.02",
    },
  });
  const text = await render(detail);
  expect(text).toContain("generated.png");
  expect(text).toContain("image/test");
  expect(text).toContain("0.02");
  expect(text).not.toContain("assistant-media:");
});

it("exports video duration and resolution instead of image parameters", async () => {
  const detail = fixture();
  detail.messages.push({
    id: "movie",
    role: "media",
    content: "",
    createdAt: 3,
    media: {
      kind: "video",
      model: "video/test",
      params: { duration: 4, resolution: "480p" },
      inputIds: [],
      outputs: [],
      status: "queued",
      cost: null,
    },
  });
  expect(await render(detail)).toContain("video/test · 4 s · 480p");
});
