import { render, screen } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { EmptyState } from "@/components/ui/empty-state";
import { KpiStrip } from "@/components/ui/kpi-strip";
import { ShareBar } from "@/components/ui/share-bar";
import { Skeleton } from "@/components/ui/skeleton";
import { Sparkline } from "@/components/ui/sparkline";
import { StackedBars } from "@/components/ui/stacked-bars";

describe("KpiStrip", () => {
  it("joins its tiles by 1px gaps inside one 12px frame", () => {
    const { container } = render(
      <KpiStrip
        items={[
          { label: "Tokens", value: "1.2M", sub: "输入 1M · 输出 200K" },
          { label: "缓存读取", value: "800K", sub: "命中率 40%" },
          { label: "花费", value: "≈$3.20" },
          { label: "请求", value: "42", title: "42 requests" },
        ]}
      />,
    );
    const frame = container.firstElementChild as HTMLElement;
    expect(frame).toHaveClass(
      "grid",
      "gap-px",
      "bg-line",
      "border",
      "border-line",
      "rounded-list",
      "overflow-hidden",
    );
    expect(frame.style.gridTemplateColumns).toBe("repeat(4, minmax(0, 1fr))");
    expect(frame.children).toHaveLength(4);
    expect(screen.getByText("1.2M")).toHaveClass(
      "text-kpi",
      "font-semibold",
      "tabular-nums",
    );
    expect(screen.getByText("Tokens")).toHaveClass(
      "text-note",
      "uppercase",
      "text-ink-muted",
    );
    expect(screen.getByText("命中率 40%")).toHaveClass("text-note");
    expect(screen.getByText("42").parentElement).toHaveAttribute(
      "title",
      "42 requests",
    );
  });
});

describe("StackedBars", () => {
  const series = [
    { start: "Mon", a: 30, b: 10 },
    { start: "Tue", a: 60, b: 20 },
    { start: "Wed", a: 0, b: 0 },
    { start: "Thu", a: 10, b: 10 },
    { start: "Fri", a: 20, b: 0 },
  ];

  it("draws one 72px-tall bar per point, output in brand over input at .38, scaled to the peak", () => {
    render(
      <StackedBars
        series={series}
        formatTitle={(p) => `${p.start}: ${p.a + p.b}`}
      />,
    );
    const bars = screen.getAllByTitle(/^(Mon|Tue|Wed|Thu|Fri): /);
    expect(bars).toHaveLength(5);
    expect(bars[0].parentElement).toHaveClass("h-[72px]");
    const [b, a] = Array.from(bars[1].children) as HTMLElement[];
    // Tue is the peak (80): its 20 output fills 25%, its 60 input 75%.
    expect(b).toHaveClass("bg-brand");
    expect(b).not.toHaveClass("opacity-[.38]");
    expect(b.style.height).toBe("25%");
    expect(a).toHaveClass("bg-brand", "opacity-[.38]");
    expect(a.style.height).toBe("75%");
    expect(bars[1]).toHaveAttribute("title", "Tue: 80");
  });

  it("labels only the first, middle and last bars", () => {
    render(
      <StackedBars series={series} formatLabel={(start) => `day ${start}`} />,
    );
    expect(screen.getByText("day Mon")).toBeInTheDocument();
    expect(screen.getByText("day Wed")).toBeInTheDocument();
    expect(screen.getByText("day Fri")).toBeInTheDocument();
    expect(screen.queryByText("day Tue")).toBeNull();
    expect(screen.queryByText("day Thu")).toBeNull();
  });
});

describe("Sparkline", () => {
  it("is a 52px SVG polyline through every point", () => {
    const { container } = render(<Sparkline points={[5, 3, 4, 1]} />);
    const svg = container.querySelector("svg")!;
    expect(svg).toHaveClass("h-[52px]");
    const points = svg
      .querySelector("polyline")!
      .getAttribute("points")!
      .trim()
      .split(/\s+/);
    expect(points).toHaveLength(4);
    expect(points[0]).toMatch(/^0,/);
    expect(points[3]).toMatch(/^100,/);
  });

  it("draws nothing without points", () => {
    const { container } = render(<Sparkline points={[]} />);
    expect(container.querySelector("polyline")).toBeNull();
  });
});

describe("ShareBar", () => {
  it("fills a 4px track to the share, held between 0 and 1", () => {
    const { container, rerender } = render(<ShareBar value={0.25} />);
    const track = container.firstElementChild as HTMLElement;
    expect(track).toHaveClass("h-1", "bg-pill", "overflow-hidden");
    const fill = () => track.firstElementChild as HTMLElement;
    expect(fill()).toHaveClass("bg-brand");
    expect(fill().style.width).toBe("25%");
    rerender(<ShareBar value={1.5} />);
    expect(fill().style.width).toBe("100%");
    rerender(<ShareBar value={Number.NaN} />);
    expect(fill().style.width).toBe("0%");
  });
});

describe("EmptyState", () => {
  it("centres a title in ink over muted text and an action", () => {
    render(
      <EmptyState title="还没有用量" action={<button>打开网页</button>}>
        用过一次之后会显示在这里
      </EmptyState>,
    );
    expect(screen.getByText("还没有用量")).toHaveClass(
      "font-medium",
      "text-ink",
    );
    expect(screen.getByText("用过一次之后会显示在这里")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "打开网页" }),
    ).toBeInTheDocument();
  });
});

describe("Skeleton", () => {
  it("is a hidden shimmering placeholder", () => {
    const { container } = render(<Skeleton className="h-3 w-20" />);
    const block = container.firstElementChild as HTMLElement;
    expect(block).toHaveAttribute("aria-hidden", "true");
    expect(block).toHaveClass("animate-skeleton", "h-3", "w-20");
  });
});
