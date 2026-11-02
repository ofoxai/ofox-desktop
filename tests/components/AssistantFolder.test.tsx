import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { AssistantFolder } from "@/components/assistant/AssistantFolder";
vi.mock("react-i18next", () => ({
  useTranslation: () => ({ t: (key: string) => key }),
}));
describe("assistant folder", () => {
  it("passes pasted paths only on explicit submission and supports the native picker", async () => {
    const onChoose = vi.fn().mockResolvedValue(true);
    render(<AssistantFolder onChoose={onChoose} />);
    fireEvent.change(screen.getByRole("textbox"), {
      target: { value: "/project/problem" },
    });
    expect(onChoose).not.toHaveBeenCalled();
    fireEvent.click(
      screen.getByRole("button", { name: "assistant.local.useFolder" }),
    );
    await waitFor(() =>
      expect(onChoose).toHaveBeenCalledWith("/project/problem"),
    );
    fireEvent.click(
      screen.getByRole("button", { name: "assistant.local.chooseFolder" }),
    );
    await waitFor(() => expect(onChoose).toHaveBeenCalledWith());
  });
});
