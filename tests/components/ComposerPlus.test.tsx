import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { ComposerPlus } from "@/components/assistant/ComposerPlus";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
const props = () => ({
  models: [],
  media: {},
  onMedia: vi.fn(),
  canAttach: true,
  onAttach: vi.fn(),
  disabled: false,
  canPresentation: true,
  onPresentation: vi.fn(),
});
const openMenu = async () => {
  const trigger = screen.getByRole("button", {
    name: "Add content or a feature",
  });
  await userEvent.click(trigger);
  return trigger;
};

describe("the contextual PPT entry in the composer", () => {
  it("opens style selection without a media hint and passes the stable plus trigger", async () => {
    const p = props();
    render(<ComposerPlus {...p} />);
    const trigger = await openMenu();
    await userEvent.click(
      await screen.findByRole("menuitem", {
        name: i18n.t("assistant.presentation.open"),
      }),
    );
    expect(p.onPresentation).toHaveBeenCalledExactlyOnceWith(trigger);
    expect(p.onMedia).not.toHaveBeenCalled();
    expect(p.onAttach).not.toHaveBeenCalled();
  });

  it("disables only the PPT entry when presentation selection is unavailable", async () => {
    const p = props();
    render(<ComposerPlus {...p} canPresentation={false} />);
    await openMenu();
    const item = await screen.findByRole("menuitem", {
      name: i18n.t("assistant.presentation.open"),
    });
    expect(item).toHaveAttribute("aria-disabled", "true");
    await userEvent.click(item);
    expect(p.onPresentation).not.toHaveBeenCalled();
    const attach = screen.getByRole("menuitem", {
      name: "Add images and files",
    });
    expect(attach).not.toHaveAttribute("aria-disabled", "true");
  });

  it("keeps the old menu unchanged when its parent offers no presentation action", async () => {
    const p = props();
    render(<ComposerPlus {...p} onPresentation={undefined} />);
    await openMenu();
    expect(
      screen.queryByRole("menuitem", {
        name: i18n.t("assistant.presentation.open"),
      }),
    ).toBeNull();
  });
});
