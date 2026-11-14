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
import en from "@/i18n/locales/en.json";
import coverage from "../../assistant-runtime/skills/ofox-office-pptx/resources/source-coverage.json";
import { PresentationStylePicker } from "@/components/assistant/PresentationStylePicker";
import { PresentationSourcePreview } from "@/components/assistant/PresentationSourcePreview";
import {
  presentationSource,
  PRESENTATION_SOURCES,
  type PresentationSource,
} from "@/lib/assistantPresentationSources";
import {
  PRESENTATION_STYLES,
  presentationStyleName,
} from "@/lib/assistantPresentationStyles";

interface CoverageStyle {
  id: string;
  sourceDeck: {
    pageCount: number;
    preview: {
      pages?: { number: number; path: string }[];
      dependencyOverrides?: { version: string; repository: string }[];
      visualFindings?: { code: string; pages: number[] }[];
    };
  };
}
const records = (coverage as unknown as { styles: CoverageStyle[] }).styles;
const record = (id: string) => records.find((item) => item.id === id);
const nameOf = (id: string) =>
  presentationStyleName(id, i18n.getFixedT("en"), "en");
/** A real upstream deck whose pages have all been rendered. */
const rendered = PRESENTATION_SOURCES.find(
  (source) =>
    source.pageCount > 2 && source.pages.every((page) => page.src !== null),
)!;

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});

describe("actual upstream presentation source pages", () => {
  it("shows the original deck as its own view of the style preview and never selects while browsing", async () => {
    const onSelect = vi.fn();
    render(
      <PresentationStylePicker
        open
        value="auto"
        onOpenChange={vi.fn()}
        onSelect={onSelect}
      />,
    );
    const name = nameOf(rendered.id);
    const entrance = screen.getByRole("button", { name: `Preview ${name}` });
    await userEvent.click(entrance);
    const viewer = await screen.findByRole("dialog", {
      name: `${name} · Style preview`,
    });
    await userEvent.click(
      within(viewer).getByRole("radio", { name: "Original deck" }),
    );
    const total = rendered.pageCount;
    expect(within(viewer).getByText(`Page 1 of ${total}`)).toBeVisible();
    expect(within(viewer).getByRole("navigation")).toHaveAccessibleName(
      "Original source pages",
    );
    expect(
      within(viewer).getAllByRole("button", { name: /^Original page / }),
    ).toHaveLength(total);
    expect(
      within(within(viewer).getByRole("figure")).getByRole("img"),
    ).toHaveAttribute(
      "src",
      expect.stringContaining(`source-previews/${rendered.id}/page-01`),
    );
    within(viewer).getByRole("figure").focus();
    await userEvent.keyboard("{End}");
    expect(within(viewer).getByText(`Page ${total} of ${total}`)).toBeVisible();
    expect(
      within(viewer).getByRole("button", { name: "Next original page" }),
    ).toBeDisabled();
    await userEvent.keyboard("{Escape}");
    expect(onSelect).not.toHaveBeenCalled();
    await waitFor(() => expect(entrance).toHaveFocus());
    expect(screen.getByRole("radio", { name: "Automatic" })).toBeChecked();
  });

  it("has a source record with exact page counts and bundled URLs for every selectable style, and none for candidates", () => {
    expect(PRESENTATION_SOURCES.map((source) => source.id)).toEqual(
      PRESENTATION_STYLES.filter((style) => record(style.id)).map(
        (style) => style.id,
      ),
    );
    expect(PRESENTATION_SOURCES.length).toBeGreaterThan(1);
    for (const source of PRESENTATION_SOURCES) {
      const entry = record(source.id)!;
      expect(source.pageCount).toBe(entry.sourceDeck.pageCount);
      expect(source.pages).toHaveLength(entry.sourceDeck.pageCount);
      const previews = entry.sourceDeck.preview.pages ?? [];
      for (const page of source.pages) {
        // A page without a rendered original stays empty: nothing stands in.
        if (previews.some((item) => item.number === page.number))
          expect(page.src).toContain(`source-previews/${source.id}/page-`);
        else expect(page.src).toBeNull();
        expect(page.src ?? "").not.toMatch(/file:|\/Users\//);
      }
      for (const link of source.licenses)
        expect(link.url).toMatch(/^https:\/\/github\.com\//);
    }
    expect(presentationSource("navy")!.pageCount).toBe(0);
    expect(presentationSource("auto")).toBeUndefined();
    expect(presentationSource("../../private")).toBeUndefined();
    const selectable = new Set(PRESENTATION_STYLES.map((style) => style.id));
    for (const entry of records.filter((item) => !selectable.has(item.id)))
      expect(presentationSource(entry.id)).toBeUndefined();
  });

  it("reports the native status from the real page mapping", () => {
    expect(presentationSource("navy")!.nativeStatus).toBe("builtin");
    expect(presentationSource("navy")!.nativeCounts).toBeNull();
    for (const source of PRESENTATION_SOURCES.filter(
      (item) => item.id !== "navy",
    )) {
      const style = PRESENTATION_STYLES.find((item) => item.id === source.id)!;
      expect(source.nativeCounts).toEqual(style.counts);
      const complete =
        style.counts !== null && style.counts.adapted === style.counts.source;
      expect(source.nativeStatus).toBe(complete ? "adapted" : "partial");
    }
  });

  it("attaches preview dependency notes and licences to exactly the decks whose coverage lists overrides", () => {
    for (const source of PRESENTATION_SOURCES) {
      const overrides =
        record(source.id)!.sourceDeck.preview.dependencyOverrides ?? [];
      expect(source.dependencies.map((item) => item.version)).toEqual(
        overrides.map((item) => item.version),
      );
      for (const dependency of overrides) {
        const name = dependency.repository.split("/").pop()!;
        expect(source.dependencies.map((item) => item.name)).toContain(name);
        expect(
          source.licenses.some((license) =>
            license.name.startsWith(`${name} ${dependency.version}`),
          ),
        ).toBe(true);
      }
    }
    expect(
      PRESENTATION_SOURCES.some((source) => source.dependencies.length > 0),
    ).toBe(
      records.some(
        (item) =>
          presentationSource(item.id) &&
          (item.sourceDeck.preview.dependencyOverrides ?? []).length > 0,
      ),
    );
  });

  it("shows true upstream rendering limitations, pinned dependencies and public license references", () => {
    const fixture: PresentationSource = {
      id: "fixture",
      kind: "html",
      pageCount: 6,
      layouts: [],
      nativeStatus: "partial",
      nativeCounts: null,
      pages: [1, 2, 3, 4, 5, 6].map((number) => ({
        number,
        label: `Upstream ${number}`,
        src: `/upstream/page-${number}.jpg`,
      })),
      sourceName: "fixture-source",
      revision: "0123456789abcdef0123456789abcdef01234567",
      sourceUrl: "https://github.com/example/decks/tree/0123/templates/fixture",
      licenses: [
        {
          name: "Chart.js 4.6.1 · MIT",
          url: "https://github.com/chartjs/Chart.js/blob/76d8cf895a7485dc838c90ef37e2bc9d9e388fcb/LICENSE.md",
        },
        {
          name: "@kurkle/color 0.3.2 · MIT",
          url: "https://github.com/kurkle/color/blob/eb0d1db2/LICENSE.md",
        },
      ],
      dependencies: [{ name: "Chart.js", version: "4.6.1" }],
      findings: [
        { code: "original-card-text-clipped", pages: [2, 4, 5] },
        { code: "original-footer-stat-overlap", pages: [2, 3, 4, 5, 6] },
      ],
    };
    render(<PresentationSourcePreview source={fixture} />);
    expect(within(screen.getByRole("figure")).getByRole("img")).toHaveClass(
      "h-full",
      "w-full",
      "object-contain",
    );
    expect(
      screen.getByText(/upstream example pages, not the native/),
    ).toBeVisible();
    expect(screen.getByText(/Original template: 6 pages/)).toHaveTextContent(
      "Native adaptation: partial",
    );
    expect(screen.getAllByRole("alert")).toHaveLength(2);
    for (const warning of screen.getAllByRole("alert"))
      expect(warning).toHaveClass("text-warn");
    expect(
      screen.getByText(/card numbers or titles are clipped.*2, 4, 5/i),
    ).toBeVisible();
    expect(
      screen.getByText(/bottom statistics and footers overlap.*2, 3, 4, 5, 6/i),
    ).toBeVisible();
    expect(screen.getByText(/local replacement fonts/)).toBeVisible();
    expect(screen.getByText(/Chart.js 4.6.1 is pinned.*unknown/)).toBeVisible();
    expect(
      screen.getByRole("link", { name: /Chart.js 4.6.1/ }),
    ).toHaveAttribute(
      "href",
      expect.stringContaining("/76d8cf895a7485dc838c90ef37e2bc9d9e388fcb/"),
    );
    expect(
      screen.getByRole("link", { name: /@kurkle\/color 0.3.2/ }),
    ).toBeInTheDocument();
    expect(document.body.innerHTML).not.toMatch(/file:\/\/|\/Users\//);
  });

  it("renders every real source record without exposing a local path", () => {
    for (const source of PRESENTATION_SOURCES) {
      const shown = render(<PresentationSourcePreview source={source} />);
      expect(
        screen.queryAllByRole("button", { name: /^Original page / }),
      ).toHaveLength(source.pageCount);
      expect(
        screen.getByText(
          `Page ${source.pageCount ? 1 : 0} of ${source.pageCount}`,
        ),
      ).toBeVisible();
      for (const dependency of source.dependencies)
        expect(
          screen.getByText(
            new RegExp(`${dependency.version} is pinned for this preview`),
          ),
        ).toBeVisible();
      expect(document.body.innerHTML).not.toMatch(/file:\/\/|\/Users\//);
      shown.unmount();
    }
  });

  it("isolates previous image events, lets an actual failed page retry, and retains keyboard page boundaries", async () => {
    render(<PresentationSourcePreview source={rendered} />);
    const figure = screen.getByRole("figure");
    const image = () => within(figure).getByRole("img");
    const total = rendered.pageCount;
    const oldImage = image();
    const next = screen.getByRole("button", { name: "Next original page" });
    await userEvent.click(next);
    const second = image();
    fireEvent.error(oldImage);
    expect(image()).toBe(second);
    fireEvent.error(second);
    expect(screen.getByRole("status")).toHaveTextContent(
      /not been replaced by a style example/,
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Reload original page" }),
    );
    expect(image()).toHaveAttribute("src", rendered.pages[1].src);
    expect(figure).toHaveFocus();
    await userEvent.click(
      screen.getByRole("button", { name: /^Original page 3 / }),
    );
    await userEvent.keyboard("{End}");
    expect(
      screen.getByRole("button", {
        name: new RegExp(`^Original page ${total} `),
      }),
    ).toHaveFocus();
    expect(next).toBeDisabled();
    await userEvent.keyboard("{Home}");
    expect(
      screen.getByRole("button", { name: /^Original page 1 / }),
    ).toHaveFocus();
    expect(
      screen.getByRole("button", { name: "Previous original page" }),
    ).toBeDisabled();
  });

  it("explains the built-in zero-content-page master instead of inventing an eight-page upstream deck", () => {
    render(<PresentationSourcePreview source={presentationSource("navy")!} />);
    expect(screen.getByText(/0 content pages · 3 layouts/)).toBeVisible();
    expect(
      screen.getByText(
        /no original content pages.*Cover \/ Section \/ Content/,
      ),
    ).toBeVisible();
    expect(screen.getByText("Page 0 of 0")).toBeVisible();
    expect(screen.queryByRole("img")).toBeNull();
    expect(
      screen.getByRole("button", { name: "Next original page" }),
    ).toBeDisabled();
  });

  it("closes the preview when gallery selection scope changes, with no selection side effect", async () => {
    const onSelect = vi.fn();
    const props = { open: true, onOpenChange: vi.fn(), onSelect };
    const { rerender } = render(
      <PresentationStylePicker {...props} value="auto" />,
    );
    const name = nameOf(rendered.id);
    await userEvent.click(
      screen.getByRole("button", { name: `Preview ${name}` }),
    );
    await userEvent.click(screen.getByRole("radio", { name: "Original deck" }));
    rerender(<PresentationStylePicker {...props} value="navy" />);
    expect(
      screen.queryByRole("dialog", { name: `${name} · Style preview` }),
    ).toBeNull();
    expect(onSelect).not.toHaveBeenCalled();
  });
});
