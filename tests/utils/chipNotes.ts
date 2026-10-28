import { act, screen, waitFor } from "@testing-library/react";
import { expect } from "vitest";

/**
 * Everything a file chip tells about its file: the text it shows, and the
 * words behind each note badge or warning icon, which sit in a tooltip that
 * keyboard focus opens.
 */
export async function chipNotes(item: HTMLElement): Promise<string> {
  const words = [item.textContent ?? ""];
  const triggers = item.querySelectorAll<HTMLElement>(
    "[tabindex='0'], button:not([data-remove-file])",
  );
  for (const trigger of triggers) {
    act(() => trigger.focus());
    words.push((await screen.findByRole("tooltip")).textContent ?? "");
    act(() => trigger.blur());
    await waitFor(() => expect(screen.queryByRole("tooltip")).toBeNull());
  }
  return words.join(" ");
}
