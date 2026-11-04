import { act, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { FoxLoader, useLoadingFox } from "@/components/ui/fox-loader";

// The fox that greets a page's first load.

beforeEach(async () => {
  vi.useFakeTimers();
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
});
afterEach(() => vi.useRealTimers());

const wait = (ms: number) => act(() => void vi.advanceTimersByTime(ms));

describe("FoxLoader", () => {
  it("renders nothing before the delay", () => {
    render(<FoxLoader />);
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("announces the generic label once shown", () => {
    render(<FoxLoader />);
    wait(200);
    expect(screen.getByRole("status", { name: "Loading…" })).toBeVisible();
  });

  it("uses the label it is given", () => {
    render(<FoxLoader label="Reading usage…" />);
    wait(200);
    expect(
      screen.getByRole("status", { name: "Reading usage…" }),
    ).toHaveTextContent("Reading usage…");
  });

  it("page size shows the progress bar", () => {
    render(<FoxLoader size="page" />);
    wait(200);
    expect(screen.getByTestId("fox-bar")).toBeInTheDocument();
  });

  it("block size has no bar", () => {
    render(<FoxLoader size="block" />);
    wait(200);
    expect(screen.queryByTestId("fox-bar")).toBeNull();
  });

  it("draws the fox with ears and two eyes", () => {
    const { container } = render(<FoxLoader />);
    wait(200);
    expect(container.querySelector('[data-part="ear-l"]')).not.toBeNull();
    expect(container.querySelector('[data-part="ear-r"]')).not.toBeNull();
    expect(container.querySelectorAll('[data-part="eye"]')).toHaveLength(2);
  });
});

describe("useLoadingFox", () => {
  it("holds the page on the fox for at least 600ms once it shows", () => {
    function Page({ pending }: { pending: boolean }) {
      const fox = useLoadingFox(pending, { label: "Reading usage…" });
      return fox.loading ? <>{fox.fox}</> : <p>content</p>;
    }
    const { rerender } = render(<Page pending />);
    wait(200);
    expect(
      screen.getByRole("status", { name: "Reading usage…" }),
    ).toBeVisible();
    wait(50);
    rerender(<Page pending={false} />);
    expect(
      screen.getByRole("status", { name: "Reading usage…" }),
    ).toBeVisible();
    expect(screen.queryByText("content")).toBeNull();
    wait(560);
    expect(screen.queryByRole("status")).toBeNull();
    expect(screen.getByText("content")).toBeVisible();
  });

  it("shows nothing and no content gap for a load under 200ms", () => {
    function Page({ pending }: { pending: boolean }) {
      const fox = useLoadingFox(pending);
      return fox.loading ? <>{fox.fox}</> : <p>content</p>;
    }
    const { rerender } = render(<Page pending />);
    wait(150);
    expect(screen.queryByRole("status")).toBeNull();
    rerender(<Page pending={false} />);
    expect(screen.getByText("content")).toBeVisible();
  });
});
