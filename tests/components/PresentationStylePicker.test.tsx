import { useRef, useState } from "react";
import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import en from "@/i18n/locales/en.json";
import ja from "@/i18n/locales/ja.json";
import { PresentationStylePicker } from "@/components/assistant/PresentationStylePicker";
import type { AssistantPresentationStyleId } from "@/lib/api/assistant";
import {
  PRESENTATION_FAMILIES,
  PRESENTATION_STYLES,
  presentationFamilyName,
  presentationStyleIntro,
  presentationStyleName,
  presentationStyleScenarios,
  type PresentationStyle,
} from "@/lib/assistantPresentationStyles";

const LOCALES = { zh, en, ja };

beforeEach(async () => {
  for (const [code, bundle] of Object.entries(LOCALES))
    i18n.addResourceBundle(code, "translation", bundle, true, true);
  await i18n.changeLanguage("en");
});

const nameOf = (style: PresentationStyle, language = "en") =>
  presentationStyleName(style.id, i18n.getFixedT(language), language);
const catalogStyles = PRESENTATION_STYLES.filter((style) => !style.builtIn);
// Any catalog style will do: the tests follow whatever is enabled.
const sample = catalogStyles[0];
const other = catalogStyles[catalogStyles.length - 1];
const last = PRESENTATION_STYLES[PRESENTATION_STYLES.length - 1];

function Harness({
  selected = "auto",
  onSelect = vi.fn(),
}: {
  selected?: AssistantPresentationStyleId;
  onSelect?: (id: AssistantPresentationStyleId) => void;
}) {
  const [open, setOpen] = useState(false);
  const [value, setValue] = useState(selected);
  const trigger = useRef<HTMLButtonElement>(null);
  return (
    <>
      <button ref={trigger} onClick={() => setOpen(true)}>
        Browse styles
      </button>
      <PresentationStylePicker
        open={open}
        onOpenChange={setOpen}
        value={value}
        returnFocusRef={trigger}
        onSelect={(id) => {
          onSelect(id);
          setValue(id);
          setOpen(false);
        }}
      />
    </>
  );
}

async function browse() {
  await userEvent.click(screen.getByRole("button", { name: "Browse styles" }));
  return screen.findByRole("dialog", { name: "Choose a presentation style" });
}
const styleGroup = () =>
  screen.getByRole("radiogroup", { name: "Presentation styles" });
const styleRadios = () => within(styleGroup()).getAllByRole("radio");
const styleRadio = (name: string) =>
  within(styleGroup()).getByRole("radio", { name });
const familyGroup = () =>
  screen.getByRole("radiogroup", { name: "Style family" });

describe("local presentation style selection", () => {
  it("offers Auto plus every selectable catalog style and commits only on Use", async () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await browse();
    expect(styleRadios()).toHaveLength(PRESENTATION_STYLES.length + 1);
    for (const style of PRESENTATION_STYLES)
      expect(styleRadio(nameOf(style))).toBeInTheDocument();
    expect(styleRadio("Automatic")).toBeChecked();
    await userEvent.click(styleRadio(nameOf(sample)));
    expect(onSelect).not.toHaveBeenCalled();
    expect(styleRadio(nameOf(sample))).toBeChecked();
    await userEvent.click(
      screen.getByRole("button", { name: "Use this style" }),
    );
    expect(onSelect).toHaveBeenCalledExactlyOnceWith(sample.id);
    await browse();
    expect(styleRadio(nameOf(sample))).toBeChecked();
  });

  it.each(["zh", "en", "ja"])(
    "shows each card's catalog name, scenarios, intro and scheme in %s with a lazy cover",
    async (language) => {
      await i18n.changeLanguage(language);
      const t = i18n.getFixedT(language);
      render(
        <PresentationStylePicker
          open
          value="auto"
          onOpenChange={vi.fn()}
          onSelect={vi.fn()}
        />,
      );
      const group = screen.getByRole("radiogroup", {
        name: t("assistant.presentation.options"),
      });
      for (const style of PRESENTATION_STYLES) {
        const card = within(group).getByRole("radio", {
          name: nameOf(style, language),
        });
        const intro = presentationStyleIntro(style, t, language);
        expect(intro.trim()).not.toBe("");
        expect(card).toHaveTextContent(intro);
        expect(card).toHaveTextContent(presentationStyleScenarios(style, t));
        expect(card).toHaveTextContent(
          style.scheme
            ? t(`assistant.presentation.schemes.${style.scheme}`)
            : t("assistant.presentation.builtIn"),
        );
        expect(card).toHaveAccessibleDescription(
          expect.stringContaining(intro),
        );
        const image = card.querySelector("img");
        if (style.cover) {
          expect(image).toHaveAttribute("src", style.cover);
          expect(image).toHaveAttribute("loading", "lazy");
        } else {
          expect(image).toBeNull();
          expect(card).toHaveTextContent(
            t("assistant.presentation.nativeUnavailable"),
          );
        }
      }
      expect(document.body.innerHTML).not.toContain("assistant.presentation.");
      expect(document.body.innerHTML).not.toMatch(/file:\/\/|\/Users\//);
    },
  );

  it("shows Auto as the first card of the style grid, not a banner above it", async () => {
    render(<Harness onSelect={vi.fn()} />);
    await browse();
    const auto = styleRadio("Automatic");
    const card = auto.parentElement!;
    const grid = card.parentElement!;
    expect(grid.firstElementChild).toBe(card);
    expect(
      within(grid).getByRole("radio", { name: nameOf(sample) }),
    ).toBeInTheDocument();
  });

  it("filters by family, combines the family with search, and never selects while filtering", async () => {
    const onSelect = vi.fn();
    render(<Harness selected={sample.id} onSelect={onSelect} />);
    await browse();
    const families = within(familyGroup()).getAllByRole("radio");
    expect(families.map((item) => item.textContent)).toEqual([
      "All",
      ...PRESENTATION_FAMILIES.map((family) =>
        presentationFamilyName(family, i18n.t),
      ),
    ]);
    expect(families[0]).toBeChecked();
    for (const family of PRESENTATION_FAMILIES) {
      await userEvent.click(
        within(familyGroup()).getByRole("radio", {
          name: presentationFamilyName(family, i18n.t),
        }),
      );
      const expected = PRESENTATION_STYLES.filter(
        (style) => style.family === family,
      );
      expect(
        styleRadios().map((item) => item.getAttribute("aria-label")),
      ).toEqual(["Automatic", ...expected.map((style) => nameOf(style))]);
    }
    // Search narrows within the chosen family and reports an empty result.
    const family = sample.family;
    await userEvent.click(
      within(familyGroup()).getByRole("radio", {
        name: presentationFamilyName(family, i18n.t),
      }),
    );
    const search = screen.getByRole("textbox", {
      name: "Search styles or scenarios",
    });
    await userEvent.type(search, sample.id);
    expect(styleRadio(nameOf(sample))).toBeVisible();
    const outside = PRESENTATION_STYLES.find(
      (style) => style.family !== family,
    );
    if (outside) {
      await userEvent.clear(search);
      await userEvent.type(search, outside.id);
      expect(
        within(styleGroup()).queryByRole("radio", { name: nameOf(outside) }),
      ).toBeNull();
      await userEvent.click(
        within(familyGroup()).getByRole("radio", { name: "All" }),
      );
      expect(styleRadio(nameOf(outside))).toBeVisible();
    }
    expect(onSelect).not.toHaveBeenCalled();
    expect(screen.getByText(`Selected: ${nameOf(sample)}`)).toBeVisible();
  });

  it("searches the catalog name, intro and scenario labels", async () => {
    render(<Harness />);
    await browse();
    const search = screen.getByRole("textbox", {
      name: "Search styles or scenarios",
    });
    const matching = (text: string) =>
      PRESENTATION_STYLES.filter((style) =>
        [
          style.id,
          nameOf(style),
          presentationStyleIntro(style, i18n.t, "en"),
          presentationStyleScenarios(style, i18n.t),
          presentationFamilyName(style.family, i18n.t),
        ]
          .join(" ")
          .toLocaleLowerCase()
          .includes(text.toLocaleLowerCase()),
      ).map((style) => nameOf(style));
    for (const text of [
      nameOf(sample),
      presentationStyleIntro(sample, i18n.t, "en"),
      presentationStyleScenarios(sample, i18n.t).split(" · ")[0],
    ]) {
      fireEvent.change(search, { target: { value: text } });
      expect(matching(text)).toContain(nameOf(sample));
      expect(
        styleRadios()
          .slice(1)
          .map((item) => item.getAttribute("aria-label")),
      ).toEqual(matching(text));
    }
  });

  it("discards uncommitted changes on Cancel and returns focus to its entrance", async () => {
    const onSelect = vi.fn();
    render(<Harness selected="navy" onSelect={onSelect} />);
    await browse();
    await userEvent.click(styleRadio(nameOf(other)));
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(screen.getByRole("button", { name: "Browse styles" })).toHaveFocus();
    expect(onSelect).not.toHaveBeenCalled();
    await browse();
    expect(styleRadio("Classic Navy")).toBeChecked();
  });

  it("supports Escape without applying a style", async () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await browse();
    await userEvent.keyboard("{Escape}");
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onSelect).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "Browse styles" })).toHaveFocus();
  });

  it("preserves the chosen style while browsing no matches", async () => {
    const onSelect = vi.fn();
    render(<Harness selected={sample.id} onSelect={onSelect} />);
    await browse();
    const search = screen.getByRole("textbox", {
      name: "Search styles or scenarios",
    });
    await userEvent.type(search, "no-matching-style-123");
    expect(styleRadios()).toHaveLength(1);
    expect(screen.getByRole("status")).toBeVisible();
    expect(screen.getByText(`Selected: ${nameOf(sample)}`)).toBeVisible();
    expect(
      screen.getByRole("button", { name: "Use this style" }),
    ).toHaveAccessibleDescription(`Selected: ${nameOf(sample)}`);
    await userEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(search).toHaveFocus();
    expect(styleRadio(nameOf(sample))).toBeChecked();
    await userEvent.click(
      screen.getByRole("button", { name: "Use this style" }),
    );
    expect(onSelect).toHaveBeenCalledExactlyOnceWith(sample.id);
  });

  it("supports radio arrow, Home, End and Space keys without committing", async () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await browse();
    styleRadio("Automatic").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(styleRadio("Classic Navy")).toHaveFocus();
    expect(styleRadio("Classic Navy")).toBeChecked();
    await userEvent.keyboard("{End}");
    expect(styleRadio(nameOf(last))).toHaveFocus();
    await userEvent.keyboard("{Home} ");
    expect(styleRadio("Automatic")).toHaveFocus();
    expect(styleRadio("Automatic")).toBeChecked();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("keeps one tab stop per group and moves with Tab and Shift+Tab between search, filter, styles and preview", async () => {
    render(<Harness />);
    await browse();
    const search = screen.getByRole("textbox", {
      name: "Search styles or scenarios",
    });
    search.focus();
    await userEvent.tab();
    expect(
      within(familyGroup()).getByRole("radio", { name: "All" }),
    ).toHaveFocus();
    await userEvent.tab();
    expect(styleRadio("Automatic")).toHaveFocus();
    await userEvent.tab();
    expect(
      screen.getByRole("button", { name: "Preview Classic Navy" }),
    ).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(styleRadio("Automatic")).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(
      within(familyGroup()).getByRole("radio", { name: "All" }),
    ).toHaveFocus();
    await userEvent.tab({ shift: true });
    expect(search).toHaveFocus();
    expect(
      styleRadios().filter((item) => item.getAttribute("tabindex") === "0"),
    ).toHaveLength(1);
  });

  it("previews a style in a nested dialog, then Escape returns to its card without selecting", async () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await browse();
    expect(screen.getAllByRole("button", { name: /^Preview / })).toHaveLength(
      PRESENTATION_STYLES.length,
    );
    const previewButton = screen.getByRole("button", {
      name: `Preview ${nameOf(sample)}`,
    });
    await userEvent.click(previewButton);
    const preview = await screen.findByRole("dialog", {
      name: `${nameOf(sample)} · Style preview`,
    });
    expect(
      within(preview).getByRole("radiogroup", { name: "Preview view" }),
    ).toBeVisible();
    expect(within(preview).getByRole("navigation")).toBeVisible();
    expect(onSelect).not.toHaveBeenCalled();
    await userEvent.keyboard("{Escape}");
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", {
          name: `${nameOf(sample)} · Style preview`,
        }),
      ).toBeNull(),
    );
    expect(
      screen.getByRole("dialog", { name: "Choose a presentation style" }),
    ).toBeVisible();
    expect(previewButton).toHaveFocus();
    expect(styleRadio("Automatic")).toBeChecked();
    expect(onSelect).not.toHaveBeenCalled();
  });

  it("applies the previewed style explicitly, independent of the prior radio choice", async () => {
    const onSelect = vi.fn();
    render(<Harness selected="navy" onSelect={onSelect} />);
    await browse();
    await userEvent.click(
      screen.getByRole("button", { name: `Preview ${nameOf(other)}` }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: `Use ${nameOf(other)}` }),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(onSelect).toHaveBeenCalledExactlyOnceWith(other.id);
    expect(screen.getByRole("button", { name: "Browse styles" })).toHaveFocus();
  });

  it("keeps selection usable when a cover image cannot load", async () => {
    const onSelect = vi.fn();
    render(<Harness onSelect={onSelect} />);
    await browse();
    const card = styleRadio("Classic Navy");
    fireEvent.error(within(card).getByRole("img"));
    expect(within(card).queryByRole("img")).toBeNull();
    await userEvent.click(card);
    await userEvent.click(
      screen.getByRole("button", { name: "Use this style" }),
    );
    expect(onSelect).toHaveBeenCalledExactlyOnceWith("navy");
  });
});
