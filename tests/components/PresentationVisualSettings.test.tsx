import { useState } from "react";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { PresentationVisualSettings } from "@/components/assistant/PresentationVisualSettings";
import {
  presentationImageChoice,
  presentationImagePolicy,
  type AssistantPresentationVisualPolicy,
} from "@/lib/api/assistant";

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  Element.prototype.scrollIntoView = vi.fn();
});
function Harness({
  onChange = vi.fn(),
  initial = { mode: "off", maxImages: 0, maxCostUsd: "0" },
}: {
  onChange?: (policy: AssistantPresentationVisualPolicy) => void;
  initial?: AssistantPresentationVisualPolicy;
}) {
  const [value, setValue] =
    useState<AssistantPresentationVisualPolicy>(initial);
  return (
    <PresentationVisualSettings
      value={value}
      onChange={(next) => {
        onChange(next);
        setValue(next);
      }}
    />
  );
}
const trigger = () => screen.getByRole("combobox", { name: "Slide images" });

const count = (name: string) => screen.queryByRole("combobox", { name });

it("defaults to no images, with no number to set; opening it changes nothing", async () => {
  const onChange = vi.fn();
  render(<Harness onChange={onChange} />);
  expect(trigger()).toHaveTextContent("Images: No images");
  expect(count("Image limit")).toBeNull();
  await userEvent.click(trigger());
  for (const label of ["No images", "Auto", "Set number"])
    expect(
      screen.getByRole("option", { name: new RegExp(label) }),
    ).toBeVisible();
  // No dollar fields or text-cost threshold to fill in.
  expect(screen.queryByRole("textbox")).toBeNull();
  expect(screen.queryByRole("checkbox")).toBeNull();
  expect(onChange).not.toHaveBeenCalled();
  await userEvent.keyboard("{Escape}");
  expect(trigger()).toHaveFocus();
});

it("auto takes a ceiling the user can change, up to eight", async () => {
  const onChange = vi.fn();
  render(<Harness onChange={onChange} />);
  await userEvent.click(trigger());
  await userEvent.click(screen.getByRole("option", { name: /Auto/ }));
  expect(onChange).toHaveBeenLastCalledWith({
    mode: "auto",
    maxImages: 3,
    maxCostUsd: "1.20",
  });
  expect(count("Image limit")).toHaveTextContent("Up to 3");
  await userEvent.click(count("Image limit")!);
  expect(screen.getAllByRole("option")).toHaveLength(8);
  await userEvent.click(screen.getByRole("option", { name: "Up to 8" }));
  expect(onChange).toHaveBeenLastCalledWith({
    mode: "auto",
    maxImages: 8,
    maxCostUsd: "3.20",
  });
});

it("a set number asks for exactly that many and keeps the number across choices", async () => {
  const onChange = vi.fn();
  render(
    <Harness
      onChange={onChange}
      initial={{ mode: "auto", maxImages: 5, maxCostUsd: "2.00" }}
    />,
  );
  await userEvent.click(trigger());
  await userEvent.click(screen.getByRole("option", { name: /Set number/ }));
  expect(onChange).toHaveBeenLastCalledWith({
    mode: "exact",
    maxImages: 5,
    maxCostUsd: "2.00",
  });
  expect(count("Number of images")).toHaveTextContent("5 images");
  await userEvent.click(trigger());
  await userEvent.click(screen.getByRole("option", { name: /No images/ }));
  expect(onChange).toHaveBeenLastCalledWith({
    mode: "off",
    maxImages: 0,
    maxCostUsd: "0",
  });
  expect(count("Number of images")).toBeNull();
});

it("reads older drafts as the nearest choice and keeps an explicit text threshold", () => {
  expect(
    presentationImageChoice({ mode: "auto", maxImages: 0, maxCostUsd: "0" }),
  ).toBe("off");
  expect(
    presentationImageChoice({
      mode: "illustration",
      maxImages: 1,
      maxCostUsd: "count-only",
    }),
  ).toBe("auto");
  expect(
    presentationImageChoice({ mode: "exact", maxImages: 3, maxCostUsd: "1.2" }),
  ).toBe("exact");
  expect(
    presentationImagePolicy("exact", {
      mode: "off",
      maxImages: 0,
      maxCostUsd: "0",
      textMaxCostUsd: "0.5",
    }),
  ).toEqual({
    mode: "exact",
    maxImages: 2,
    maxCostUsd: "0.80",
    textMaxCostUsd: "0.5",
  });
  // Never more than the Host accepts, never fewer than one.
  expect(presentationImagePolicy("auto", undefined, 40).maxImages).toBe(8);
  expect(presentationImagePolicy("auto", undefined, 0).maxImages).toBe(1);
});
