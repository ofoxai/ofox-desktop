import {
  createEvent,
  fireEvent,
  render,
  screen,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import {
  PRESENTATION_STYLES,
  presentationArchetypeName,
  presentationStyleName,
  type PresentationStyle,
} from "@/lib/assistantPresentationStyles";
import {
  presentationSource,
  type PresentationSource,
} from "@/lib/assistantPresentationSources";
import { PresentationStylePreview } from "@/components/assistant/PresentationStylePreview";

const kinds = [
  "cover",
  "outline",
  "content",
  "comparison",
  "chart",
  "process",
  "equation",
  "closing",
] as const;
const base = {
  family: "business",
  scheme: null,
  scenarios: ["general"],
  names: null,
  routed: false,
  fallbackName: "Fixture",
  fallbackIntro: "",
};
/** The built-in master: eight self-written pages and no upstream deck. */
const builtIn: PresentationStyle = {
  ...base,
  id: "navy",
  builtIn: true,
  intro: null,
  cover: "/fixture/cover.jpg",
  pages: kinds.map((kind) => ({
    kind,
    src: `/fixture/${kind}.jpg`,
    thumbnail: `/fixture/${kind}-thumb.jpg`,
  })),
  nativePages: null,
  nativeSample: false,
  counts: null,
};
/** Five source pages: two adapted, two approximate, one refused. */
const adapted: PresentationStyle = {
  ...base,
  id: "fixture-adapted",
  builtIn: false,
  intro: { en: "Fixture intro for an adapted style." },
  cover: "/native/page-1.jpg",
  pages: [],
  nativePages: [
    {
      sourcePage: 1,
      archetype: "cover",
      status: "adapted",
      differences: "",
      src: "/native/page-1.jpg",
    },
    {
      sourcePage: 2,
      archetype: "split-image",
      status: "approximate",
      differences: "The image region is filled only by a planned asset.",
      src: "/native/page-2.jpg",
    },
    {
      sourcePage: 3,
      archetype: "list",
      status: "unsupported",
      differences: "There is no native fading display list.",
      src: null,
    },
    {
      sourcePage: 4,
      archetype: "stats",
      status: "approximate",
      differences: "",
      src: "/native/page-4.jpg",
    },
    {
      sourcePage: 5,
      archetype: "closing",
      status: "adapted",
      differences: "",
      src: "/native/page-5.jpg",
    },
  ],
  nativeSample: true,
  counts: { source: 5, adapted: 2, approximate: 2, unsupported: 1 },
};
/** Mapped in the catalog, but no native sample has been published yet. */
const unpublished: PresentationStyle = {
  ...adapted,
  id: "fixture-unpublished",
  cover: null,
  nativePages: adapted.nativePages!.map((page) => ({ ...page, src: null })),
  nativeSample: false,
};
const upstream: PresentationSource = {
  id: "fixture-adapted",
  kind: "html",
  pageCount: 5,
  layouts: [],
  nativeStatus: "partial",
  nativeCounts: adapted.counts,
  pages: [1, 2, 3, 4, 5].map((number) => ({
    number,
    label: `Upstream ${number}`,
    src: `/upstream/page-${number}.jpg`,
  })),
  sourceName: "fixture-source",
  revision: null,
  sourceUrl: null,
  licenses: [],
  dependencies: [],
  findings: [],
};
const props = (style = builtIn, source: PresentationSource | null = null) => ({
  style,
  source,
  name: "Fixture",
  open: true,
  onClose: vi.fn(),
  onUse: vi.fn(),
});
const viewer = () =>
  screen.getByRole("dialog", { name: "Fixture · Style preview" });
const caption = () => within(viewer()).getByText(/Page \d+ of \d+/);
const mainImage = () =>
  within(within(viewer()).getByRole("figure")).getByRole("img");
const view = (name: string) =>
  within(
    within(viewer()).getByRole("radiogroup", { name: "Preview view" }),
  ).getByRole("radio", { name });

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  Element.prototype.scrollIntoView = vi.fn();
});

describe("the built-in master's self-written example", () => {
  it("renders one fitted full page and eight small navigable thumbnails", () => {
    render(<PresentationStylePreview {...props()} />);
    const dialog = viewer();
    expect(view("Native adaptation")).toBeChecked();
    expect(within(dialog).getAllByRole("img")).toHaveLength(1);
    expect(mainImage()).toHaveAttribute("src", "/fixture/cover.jpg");
    expect(mainImage()).not.toHaveAttribute("loading", "lazy");
    expect(mainImage()).toHaveClass("h-full", "w-full", "object-contain");
    const navigation = within(dialog).getByRole("navigation", {
      name: "Sample pages",
    });
    const buttons = within(navigation).getAllByRole("button");
    expect(buttons).toHaveLength(8);
    expect(buttons[0]).toHaveAttribute("aria-current", "page");
    expect(buttons[0]).toHaveAttribute("tabindex", "0");
    expect(buttons[1]).toHaveAttribute("tabindex", "-1");
    const thumbnails = navigation.querySelectorAll("img");
    expect(thumbnails).toHaveLength(8);
    for (const image of thumbnails) {
      expect(image).toHaveAttribute("loading", "lazy");
      expect(image).toHaveAttribute("alt", "");
      expect(image).toHaveAttribute(
        "src",
        expect.stringContaining("-thumb.jpg"),
      );
    }
    expect(caption()).toHaveTextContent("Page 1 of 8");
    expect(
      within(dialog).getByRole("button", { name: "Previous page" }),
    ).toBeDisabled();
    // A self-written example is neither an adaptation nor an upstream deck.
    expect(
      within(dialog).getByText(/Eight independently authored sample slides/),
    ).toBeVisible();
    expect(within(dialog).queryByText(/source pages ·/)).toBeNull();
    expect(within(dialog).queryByText("Adapted")).toBeNull();
  });

  it("moves through every page without committing a style and respects first/last boundaries", async () => {
    const p = props();
    render(<PresentationStylePreview {...p} />);
    const next = screen.getByRole("button", { name: "Next page" });
    for (let number = 2; number <= 8; number += 1) {
      await userEvent.click(next);
      expect(caption()).toHaveTextContent(`Page ${number} of 8`);
      expect(mainImage()).toHaveAttribute("src", builtIn.pages[number - 1].src);
    }
    expect(next).toBeDisabled();
    expect(screen.getByRole("figure")).toHaveFocus();
    await userEvent.keyboard("{ArrowRight}");
    expect(caption()).toHaveTextContent("Page 8 of 8");
    const previous = screen.getByRole("button", { name: "Previous page" });
    previous.focus();
    await userEvent.keyboard("{Home}");
    expect(caption()).toHaveTextContent("Page 1 of 8");
    expect(previous).toBeDisabled();
    expect(screen.getByRole("figure")).toHaveFocus();
    await userEvent.keyboard("{ArrowLeft}");
    expect(caption()).toHaveTextContent("Page 1 of 8");
    expect(p.onUse).not.toHaveBeenCalled();
    expect(p.onClose).not.toHaveBeenCalled();
  });

  it("supports thumbnail arrows, Home and End while retaining a single current page", async () => {
    render(<PresentationStylePreview {...props()} />);
    const third = screen.getByRole("button", { name: "Page 3 · Key points" });
    await userEvent.click(third);
    expect(third).toHaveFocus();
    expect(caption()).toHaveTextContent("Page 3 of 8");
    await userEvent.keyboard("{ArrowDown}");
    const fourth = screen.getByRole("button", { name: "Page 4 · Comparison" });
    expect(fourth).toHaveFocus();
    expect(fourth).toHaveAttribute("aria-current", "page");
    expect(third).not.toHaveAttribute("aria-current");
    await userEvent.keyboard("{End}");
    expect(
      screen.getByRole("button", { name: "Page 8 · Closing" }),
    ).toHaveFocus();
    await userEvent.keyboard("{Home}");
    expect(
      screen.getByRole("button", { name: "Page 1 · Cover" }),
    ).toHaveFocus();
  });

  it("does not intercept input editing, modified keys or keys outside this dialog", () => {
    render(<PresentationStylePreview {...props()} />);
    const figure = screen.getByRole("figure");
    const modified = createEvent.keyDown(figure, {
      key: "ArrowRight",
      ctrlKey: true,
    });
    fireEvent(figure, modified);
    expect(modified.defaultPrevented).toBe(false);
    fireEvent.keyDown(window, { key: "End" });
    const input = document.createElement("input");
    figure.append(input);
    const editing = createEvent.keyDown(input, { key: "ArrowRight" });
    fireEvent(input, editing);
    expect(editing.defaultPrevented).toBe(false);
    expect(caption()).toHaveTextContent("Page 1 of 8");
    input.remove();
  });

  it("keeps loading and errors bound to the current image, and retries only that page", async () => {
    const p = props();
    render(<PresentationStylePreview {...p} />);
    const old = mainImage();
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    const current = mainImage();
    expect(current).not.toBe(old);
    fireEvent.error(old);
    fireEvent.load(old);
    expect(mainImage()).toBe(current);
    expect(screen.getByText("Loading page…")).toBeVisible();
    fireEvent.error(current);
    expect(
      screen.getByText(/This sample could not be displayed/),
    ).toBeVisible();
    await userEvent.click(
      screen.getByRole("button", { name: "Retry this page" }),
    );
    expect(mainImage()).toHaveAttribute("src", "/fixture/outline.jpg");
    expect(screen.getByRole("figure")).toHaveFocus();
    fireEvent.load(mainImage());
    expect(screen.queryByText("Loading page…")).toBeNull();
    expect(caption()).toHaveTextContent("Page 2 of 8");
    fireEvent.error(mainImage());
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(mainImage()).toHaveAttribute("src", "/fixture/content.jpg");
    fireEvent.error(mainImage());
    await userEvent.click(screen.getByRole("button", { name: "Use Fixture" }));
    expect(p.onUse).toHaveBeenCalledOnce();
    expect(p.onClose).not.toHaveBeenCalled();
  });

  it("keeps a failed thumbnail selectable and scrolls only its own navigation strip", async () => {
    render(<PresentationStylePreview {...props()} />);
    const nav = screen.getByRole("navigation", { name: "Sample pages" });
    const last = screen.getByRole("button", { name: "Page 8 · Closing" });
    fireEvent.error(last.querySelector("img")!);
    expect(last.querySelector("img")).toBeNull();
    vi.spyOn(nav, "getBoundingClientRect").mockReturnValue({
      top: 0,
      bottom: 100,
      left: 0,
      right: 112,
      width: 112,
      height: 100,
    } as DOMRect);
    vi.spyOn(last, "getBoundingClientRect").mockReturnValue({
      top: 240,
      bottom: 320,
      left: 0,
      right: 112,
      width: 112,
      height: 80,
    } as DOMRect);
    await userEvent.click(last);
    expect(caption()).toHaveTextContent("Page 8 of 8");
    expect(nav.scrollTop).toBe(220);
    expect(Element.prototype.scrollIntoView).not.toHaveBeenCalled();
  });

  it("derives the page count from metadata and resets on reopening or changing styles", async () => {
    const p = props();
    const short = { ...builtIn, pages: builtIn.pages.slice(0, 3) };
    const shown = render(<PresentationStylePreview {...p} style={short} />);
    await userEvent.keyboard("{End}");
    expect(caption()).toHaveTextContent("Page 3 of 3");
    shown.rerender(
      <PresentationStylePreview {...p} style={short} open={false} />,
    );
    expect(screen.queryByRole("dialog")).toBeNull();
    shown.rerender(<PresentationStylePreview {...p} style={short} />);
    expect(caption()).toHaveTextContent("Page 1 of 3");
    await userEvent.keyboard("{End}");
    shown.rerender(
      <PresentationStylePreview {...p} style={{ ...builtIn, id: "other" }} />,
    );
    expect(caption()).toHaveTextContent("Page 1 of 8");
  });

  it("requests closure for Escape or Back without committing a style", async () => {
    const p = props();
    render(<PresentationStylePreview {...p} />);
    await userEvent.keyboard("{Escape}");
    expect(p.onClose).toHaveBeenCalledOnce();
    expect(p.onUse).not.toHaveBeenCalled();
    await userEvent.click(
      screen.getByRole("button", { name: "Back to styles" }),
    );
    expect(p.onClose).toHaveBeenCalledTimes(2);
    expect(p.onUse).not.toHaveBeenCalled();
  });

  it("explains in the original view that a built-in master has no upstream deck", async () => {
    render(
      <PresentationStylePreview
        {...props(builtIn, presentationSource("navy")!)}
      />,
    );
    await userEvent.click(view("Original deck"));
    expect(view("Original deck")).toBeChecked();
    expect(screen.getByText(/0 content pages · 3 layouts/)).toBeVisible();
    expect(screen.getByText(/no original content pages/)).toBeVisible();
    expect(within(viewer()).queryByRole("img")).toBeNull();
  });
});

describe("a catalog style's native adaptation and original deck", () => {
  it("shows native sample pages in source order with page kind, status and real counts", async () => {
    render(<PresentationStylePreview {...props(adapted, upstream)} />);
    const dialog = viewer();
    expect(view("Native adaptation")).toBeChecked();
    expect(
      within(dialog).getByText("Fixture intro for an adapted style."),
    ).toBeVisible();
    expect(
      within(dialog).getByText(/Ofox built with its own sample content/),
    ).toBeVisible();
    expect(
      within(dialog).getByText(
        "5 source pages · 2 adapted · 2 approximate · 1 unsupported",
      ),
    ).toBeVisible();
    const navigation = within(dialog).getByRole("navigation", {
      name: "Sample pages",
    });
    expect(
      within(navigation)
        .getAllByRole("button")
        .map((button) => button.getAttribute("aria-label")),
    ).toEqual([
      "Source page 1 · Cover · Adapted",
      "Source page 2 · Text with image · Approximate",
      "Source page 4 · Key figures · Approximate",
      "Source page 5 · Closing · Adapted",
    ]);
    expect(mainImage()).toHaveAttribute("src", "/native/page-1.jpg");
    // The page itself carries no status or difference caption under it.
    expect(within(dialog).queryByTestId("presentation-native-page")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Next page" }));
    expect(caption()).toHaveTextContent("Page 2 of 4");
    expect(mainImage()).toHaveAttribute("src", "/native/page-2.jpg");
    expect(within(dialog).queryByTestId("presentation-native-page")).toBeNull();
    expect(
      within(dialog).queryByText(/Difference from the original/),
    ).toBeNull();
  });

  it("lists unsupported source pages as not adapted and never shows them as a native page", () => {
    render(<PresentationStylePreview {...props(adapted, upstream)} />);
    const list = within(viewer()).getByRole("list", {
      name: "Source pages not adapted: 1",
    });
    const items = within(list).getAllByRole("listitem");
    expect(items).toHaveLength(1);
    expect(items[0]).toHaveTextContent(
      "Source page 3 · List · Unsupported — There is no native fading display list.",
    );
    const navigation = within(viewer()).getByRole("navigation", {
      name: "Sample pages",
    });
    expect(
      within(navigation).queryByRole("button", { name: /Source page 3/ }),
    ).toBeNull();
    expect(navigation.querySelectorAll("img")).toHaveLength(4);
  });

  it("keeps the upstream pages in a separately labelled view with every source page", async () => {
    const p = props(adapted, upstream);
    render(<PresentationStylePreview {...p} />);
    await userEvent.click(view("Original deck"));
    const dialog = viewer();
    expect(view("Original deck")).toBeChecked();
    expect(
      within(dialog).getByText(/upstream example pages, not the native/),
    ).toBeVisible();
    expect(
      within(dialog).queryByText(/Ofox built with its own sample content/),
    ).toBeNull();
    expect(
      within(dialog).getAllByRole("button", { name: /^Original page / }),
    ).toHaveLength(5);
    expect(mainImage()).toHaveAttribute("src", "/upstream/page-1.jpg");
    expect(
      within(dialog).getByText(
        /5 source pages · 2 adapted · 2 approximate · 1 unsupported/,
      ),
    ).toBeVisible();
    within(dialog).getByRole("figure").focus();
    await userEvent.keyboard("{End}");
    expect(within(dialog).getByText("Page 5 of 5")).toBeVisible();
    await userEvent.click(view("Native adaptation"));
    expect(mainImage()).toHaveAttribute("src", "/native/page-1.jpg");
    expect(p.onUse).not.toHaveBeenCalled();
    expect(p.onClose).not.toHaveBeenCalled();
  });

  it("opens on the original deck and says so plainly when no native sample exists yet", async () => {
    render(<PresentationStylePreview {...props(unpublished, upstream)} />);
    expect(view("Original deck")).toBeChecked();
    expect(mainImage()).toHaveAttribute("src", "/upstream/page-1.jpg");
    await userEvent.click(view("Native adaptation"));
    const dialog = viewer();
    expect(
      within(dialog).getByText(
        "A native sample is not available for this style yet.",
      ),
    ).toBeVisible();
    expect(within(dialog).queryByRole("img")).toBeNull();
    expect(within(dialog).queryByRole("navigation")).toBeNull();
    expect(
      within(dialog).getByText(
        "5 source pages · 2 adapted · 2 approximate · 1 unsupported",
      ),
    ).toBeVisible();
    expect(
      within(dialog).getByRole("button", { name: "Use Fixture" }),
    ).toBeEnabled();
  });

  it("degrades to plain notes when a style has neither a mapping nor an original deck record", async () => {
    render(
      <PresentationStylePreview
        {...props({ ...unpublished, nativePages: null, counts: null }, null)}
      />,
    );
    const dialog = viewer();
    expect(view("Native adaptation")).toBeChecked();
    expect(
      within(dialog).getByText(
        "A native sample is not available for this style yet.",
      ),
    ).toBeVisible();
    expect(within(dialog).queryByText(/source pages ·/)).toBeNull();
    await userEvent.click(view("Original deck"));
    expect(
      within(dialog).getByText("No original deck is recorded for this style."),
    ).toBeVisible();
    expect(within(dialog).queryByRole("img")).toBeNull();
  });

  it("switches views with the keyboard and keeps Tab inside the preview", async () => {
    render(<PresentationStylePreview {...props(adapted, upstream)} />);
    expect(screen.getByRole("figure")).toHaveFocus();
    view("Native adaptation").focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(view("Original deck")).toBeChecked();
    expect(view("Original deck")).toHaveFocus();
    expect(
      within(viewer()).getAllByRole("button", { name: /^Original page / }),
    ).toHaveLength(5);
    await userEvent.keyboard("{ArrowLeft}");
    expect(view("Native adaptation")).toBeChecked();
    await userEvent.tab({ shift: true });
    expect(viewer()).toContainElement(document.activeElement as HTMLElement);
    await userEvent.tab();
    await userEvent.tab();
    expect(viewer()).toContainElement(document.activeElement as HTMLElement);
  });

  it("states the real counts for every published style and never draws an unsupported page", () => {
    const t = i18n.getFixedT("en");
    for (const style of PRESENTATION_STYLES.filter((item) => item.counts)) {
      const name = presentationStyleName(style.id, t, "en");
      const shown = render(
        <PresentationStylePreview
          {...props(style, presentationSource(style.id) ?? null)}
          name={name}
        />,
      );
      const dialog = screen.getByRole("dialog", {
        name: `${name} · Style preview`,
      });
      const counts = style.counts!;
      expect(
        within(dialog).getByText(
          new RegExp(
            `${counts.source} source pages · ${counts.adapted} adapted · ${counts.approximate} approximate · ${counts.unsupported} unsupported`,
          ),
        ),
      ).toBeVisible();
      const drawn = style.nativePages!.filter((page) => page.src);
      if (drawn.length) {
        expect(
          within(within(dialog).getByRole("navigation"))
            .getAllByRole("button")
            .map((button) => button.getAttribute("aria-label")),
        ).toEqual(
          drawn.map(
            (page) =>
              `Source page ${page.sourcePage} · ${presentationArchetypeName(page.archetype, t)} · ${page.status === "adapted" ? "Adapted" : "Approximate"}`,
          ),
        );
        const refused = style.nativePages!.filter(
          (page) => page.status === "unsupported",
        );
        expect(drawn.some((page) => page.status === "unsupported")).toBe(false);
        if (refused.length)
          expect(
            within(
              within(dialog).getByRole("list", {
                name: `Source pages not adapted: ${refused.length}`,
              }),
            ).getAllByRole("listitem"),
          ).toHaveLength(refused.length);
      }
      shown.unmount();
    }
  });
});
