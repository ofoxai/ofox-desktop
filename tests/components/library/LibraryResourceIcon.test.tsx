import { render, screen } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { Code2, Languages, ScanSearch } from "lucide-react";
import {
  LibraryResourceIcon,
  letterTile,
  TEMPLATE_ICONS,
} from "@/components/library/LibraryResourceIcon";
import { LibraryResourceRow } from "@/components/library/LibraryResourceRow";
import { LIBRARY_RECOMMENDATIONS } from "@/components/library/libraryRecommendations";

describe("library resource icons", () => {
  it("draws the same letter tile for the same name", () => {
    expect(letterTile("sentry")).toEqual(letterTile("sentry"));
    expect(letterTile("sentry").letter).toBe("S");
  });

  it.each([
    ["", "?"],
    ["  ", "?"],
    ["资源", "资"],
    ["🦊 tools", "🦊"],
    ["  sentry  ", "S"],
  ])("uses the first Unicode character of %j", (name, expected) => {
    expect(letterTile(name).letter).toBe(expected);
  });

  it("offers at least eight deterministic light and dark color pairs", () => {
    const colors = new Set(
      Array.from(
        { length: 128 },
        (_, index) => letterTile(String.fromCodePoint(65 + index)).className,
      ),
    );
    expect(colors.size).toBeGreaterThanOrEqual(8);
    for (const color of colors) {
      expect(color).toMatch(/(?:^| )bg-/);
      expect(color).toMatch(/(?:^| )text-/);
      expect(color).toMatch(/(?:^| )dark:bg-/);
      expect(color).toMatch(/(?:^| )dark:text-/);
    }
  });

  it.each(["mcp", "skills", "instructions"] as const)(
    "shows a letter tile for an unknown %s with a name",
    (kind) => {
      const { container } = render(
        <LibraryResourceIcon kind={kind} recipeId="unknown" name="my-server" />,
      );
      expect(screen.getByText("M")).toBeInTheDocument();
      expect(container.querySelector("svg, img")).toBeNull();
    },
  );

  it.each([
    ["coding", Code2, "lucide-code-xml"],
    ["chinese", Languages, "lucide-languages"],
    ["review", ScanSearch, "lucide-scan-search"],
  ] as const)(
    "shows the actual %s template icon before a letter",
    (id, icon, svgClass) => {
      expect(
        LIBRARY_RECOMMENDATIONS.find((recipe) => recipe.id === id)?.kind,
      ).toBe("instructions");
      expect(TEMPLATE_ICONS[id].icon).toBe(icon);
      const { container } = render(
        <LibraryResourceIcon
          kind="instructions"
          recipeId={id}
          name="Template"
        />,
      );
      expect(container.querySelector(`svg.${svgClass}`)).not.toBeNull();
      for (const className of TEMPLATE_ICONS[id].className.split(" ")) {
        expect(container.firstElementChild).toHaveClass(className);
      }
      expect(container.firstElementChild?.className).toContain("dark:bg-");
      expect(container.firstElementChild?.className).toContain("dark:text-");
      expect(screen.queryByText("T")).not.toBeInTheDocument();
    },
  );

  it("does not use instruction icons for other resource kinds", () => {
    render(<LibraryResourceIcon kind="mcp" recipeId="review" name="review" />);
    expect(screen.getByText("R")).toBeInTheDocument();
  });

  it("keeps a known logo before a name and preserves inversion", () => {
    const { container, rerender } = render(
      <LibraryResourceIcon kind="mcp" recipeId="context7" name="Context7" />,
    );
    expect(container.querySelector("img")).not.toBeNull();
    expect(screen.queryByText("C")).not.toBeInTheDocument();
    rerender(
      <LibraryResourceIcon kind="mcp" recipeId="github" name="GitHub" />,
    );
    expect(container.querySelector("img")).toHaveClass("dark:invert");
  });

  it.each([
    ["mcp", "lucide-plug"],
    ["skills", "lucide-puzzle"],
    ["instructions", "lucide-file-text"],
  ] as const)(
    "keeps the original fallback for a nameless %s",
    (kind, iconClass) => {
      const { container } = render(<LibraryResourceIcon kind={kind} />);
      expect(container.querySelector(`svg.${iconClass}`)).not.toBeNull();
    },
  );

  it("renders a blank supplied name as ? and keeps caller sizing and decoration", () => {
    const { container } = render(
      <LibraryResourceIcon kind="mcp" name="  " className="h-9 w-9" />,
    );
    expect(screen.getByText("?")).toBeInTheDocument();
    expect(container.firstElementChild).toHaveClass("h-9", "w-9");
    expect(container.firstElementChild).toHaveAttribute("aria-hidden", "true");
  });

  it.each([undefined, "review"])(
    "uses the shared icon in instruction rows with recipe %j",
    (recipeId) => {
      const { container } = render(
        <LibraryResourceRow
          kind="instructions"
          recipeId={recipeId}
          row={{ id: "rules", name: "Shared", content: "Rules", revision: 1 }}
          targets={[]}
          busy={false}
          writable
          selecting={false}
          selected={false}
          onSelected={vi.fn()}
          onToggle={vi.fn()}
          onAction={vi.fn()}
        />,
      );
      if (recipeId) {
        expect(
          container.querySelector("svg.lucide-scan-search"),
        ).not.toBeNull();
      } else {
        expect(screen.getByText("S")).toBeInTheDocument();
      }
    },
  );
});

it("uses only a bundled owner avatar and keeps unknown authors on letter tiles", () => {
  const { container, rerender } = render(
    <LibraryResourceIcon
      kind="skills"
      owner="vercel-labs"
      name="vercel-labs"
    />,
  );
  expect(container.querySelector("img")?.getAttribute("src")).toContain(
    "owners/vercel-labs.png",
  );
  expect(container.querySelector("img")?.getAttribute("src")).not.toMatch(
    /^https?:/,
  );
  rerender(
    <LibraryResourceIcon
      kind="skills"
      owner="unknown-author"
      name="unknown-author"
    />,
  );
  expect(container.querySelector("img")).toBeNull();
  expect(screen.getByText("U")).toBeVisible();
});
