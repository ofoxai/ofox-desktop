import { render, screen } from "@testing-library/react";
import { createInstance } from "i18next";
import { I18nextProvider } from "react-i18next";
import { describe, expect, it } from "vitest";
import { AssistantMessageView } from "@/components/assistant/AssistantMessageView";
import { assistantErrorKey } from "@/lib/api/assistant";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import zh from "@/i18n/locales/zh.json";

const languages = [
  {
    language: "zh",
    resource: zh,
    text: "费用信息不完整，已暂停以避免继续产生不确定费用。请查看请求记录后再决定是否重试。",
  },
  {
    language: "en",
    resource: en,
    text: "Cost information is incomplete. Paused to avoid further uncertain charges. Review the request records before deciding whether to retry.",
  },
  {
    language: "ja",
    resource: ja,
    text: "費用情報が不完全なため、不確かな料金がさらに発生しないよう一時停止しました。リクエスト記録を確認してから、再試行するか判断してください。",
  },
];

// The same public error code covers both task types and both admission and
// post-response failures. A network reset without usage is not missing pricing.
const failures = [
  { task: "MCP", transport: "ECONNRESET", usage: null },
  { task: "local configuration", transport: "ECONNRESET", usage: null },
  { task: "preflight", price: null },
];

describe.each(languages)(
  "incomplete cost information in $language",
  (locale) => {
    it.each(failures)(
      "does not invent the cause for $task",
      async (failure) => {
        const code = "assistant_cost_unavailable";
        const key = assistantErrorKey(code);
        expect(key).toBe("assistant.errors.localCostUnavailable");
        const i18n = createInstance();
        await i18n.init({
          lng: locale.language,
          resources: {
            [locale.language]: { translation: locale.resource },
          },
          interpolation: { escapeValue: false },
        });
        render(
          <I18nextProvider i18n={i18n}>
            <AssistantMessageView
              message={{
                id: failure.task,
                role: "assistant",
                content: "",
                createdAt: 1,
                status: "error",
                errorCode: code,
              }}
              disabled
              onEdit={async () => false}
              onRegenerate={() => undefined}
            />
          </I18nextProvider>,
        );
        expect(screen.getByText(locale.text)).toBeVisible();
        expect(screen.queryByText(code)).not.toBeInTheDocument();
      },
    );
  },
);
