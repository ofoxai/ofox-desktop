import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { AssistantContextUsage } from "@/components/assistant/AssistantContextUsage";
import { TooltipProvider } from "@/components/ui/tooltip";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
it("shows the measured usage and offers a near-full compaction hint on keyboard focus", async () => {
  const user = userEvent.setup();
  render(
    <TooltipProvider delayDuration={0}>
      <AssistantContextUsage
        modelId="model"
        usage={{ modelId: "model", tokens: 900, contextWindow: 1000 }}
      />
    </TooltipProvider>,
  );
  expect(screen.getByRole("meter")).toHaveAttribute("aria-valuenow", "90");
  await user.tab();
  expect(await screen.findByRole("tooltip")).toHaveTextContent("900 / 1,000");
  expect(screen.getByRole("tooltip")).toHaveTextContent("Compact context");
});
it("keeps unknown post-compaction usage and an old model's receipt unknown", () => {
  const view = render(
    <TooltipProvider>
      <AssistantContextUsage
        modelId="model"
        usage={{ modelId: "model", tokens: null, contextWindow: 1000 }}
      />
    </TooltipProvider>,
  );
  expect(screen.queryByRole("meter")).not.toBeInTheDocument();
  expect(
    screen.getByLabelText("Context usage unavailable"),
  ).toBeInTheDocument();
  view.rerender(
    <TooltipProvider>
      <AssistantContextUsage
        modelId="other"
        usage={{ modelId: "model", tokens: 900, contextWindow: 1000 }}
      />
    </TooltipProvider>,
  );
  expect(screen.queryByRole("meter")).not.toBeInTheDocument();
});
