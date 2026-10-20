import { act, render } from "@testing-library/react";
import { useRef } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useFitPopover } from "@/components/tray/useFitPopover";

const mocks = vi.hoisted(() => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke }));

let resize: () => void = () => undefined;
const sizes = { card: 620, view: 400, content: 120 };

beforeEach(() => {
  mocks.invoke.mockReset().mockResolvedValue(undefined);
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(callback: () => void) {
        resize = callback;
      }
      observe() {}
      disconnect() {}
    },
  );
  vi.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
    cb(0);
    return 0;
  });
});
afterEach(() => vi.unstubAllGlobals());

function Popover() {
  const card = useRef<HTMLDivElement>(null);
  const view = useRef<HTMLDivElement>(null);
  const content = useRef<HTMLDivElement>(null);
  useFitPopover(card, view, content);
  return (
    <div ref={card}>
      <div ref={view}>
        <div ref={content} />
      </div>
    </div>
  );
}

function measured(container: HTMLElement) {
  const card = container.firstElementChild as HTMLElement;
  const view = card.firstElementChild as HTMLElement;
  const content = view.firstElementChild as HTMLElement;
  Object.defineProperty(card, "offsetHeight", { get: () => sizes.card });
  Object.defineProperty(view, "clientHeight", { get: () => sizes.view });
  Object.defineProperty(content, "offsetHeight", { get: () => sizes.content });
}

function lastHeight() {
  return mocks.invoke.mock.calls.at(-1)?.[1]?.height;
}

it("asks the window for the card's height with the list at its natural size", () => {
  const { container } = render(<Popover />);
  measured(container);
  act(() => resize());
  // 620 - 400 = what is around the list; 120 = the list itself.
  expect(mocks.invoke).toHaveBeenLastCalledWith("fit_tray_popover", {
    height: 340,
  });
});

it("grows with what it shows but never shrinks while it is open", () => {
  const { container } = render(<Popover />);
  measured(container);
  act(() => resize());
  sizes.content = 260;
  act(() => resize());
  expect(lastHeight()).toBe(480);
  sizes.content = 120;
  act(() => resize());
  expect(lastHeight()).toBe(480);
  // Opened again, it starts from what it shows now.
  act(() => {
    window.dispatchEvent(new Event("focus"));
  });
  expect(lastHeight()).toBe(340);
});
