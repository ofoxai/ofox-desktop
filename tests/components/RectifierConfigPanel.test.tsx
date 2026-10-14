import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import i18n from "i18next";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";
import { RectifierConfigPanel } from "@/components/settings/RectifierConfigPanel";
import zh from "@/i18n/locales/zh.json";
import { settingsApi } from "@/lib/api/settings";

beforeAll(() => {
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  // cmdk scrolls the highlighted option into view; jsdom has no layout.
  Element.prototype.scrollIntoView = vi.fn();
});

afterEach(() => vi.restoreAllMocks());

describe("RectifierConfigPanel", () => {
  it("picks the cache TTL from the app's own menu and saves it", async () => {
    vi.spyOn(settingsApi, "getRectifierConfig").mockResolvedValue({
      enabled: true,
      requestThinkingSignature: true,
      requestThinkingBudget: true,
    });
    vi.spyOn(settingsApi, "getOptimizerConfig").mockResolvedValue({
      enabled: true,
      thinkingOptimizer: true,
      cacheInjection: true,
      cacheTtl: "1h",
    });
    const save = vi
      .spyOn(settingsApi, "setOptimizerConfig")
      .mockResolvedValue(true);
    const user = userEvent.setup();
    render(<RectifierConfigPanel />);

    const ttl = await screen.findByRole("combobox", { name: "Cache TTL" });
    expect(ttl).toHaveAttribute("aria-haspopup", "listbox");
    expect(ttl).toHaveTextContent("1 小时");

    await user.click(ttl);
    await user.click(screen.getByRole("option", { name: "5 分钟" }));
    await waitFor(() =>
      expect(save).toHaveBeenCalledWith(
        expect.objectContaining({ cacheTtl: "5m" }),
      ),
    );
    expect(ttl).toHaveTextContent("5 分钟");
  });
});
