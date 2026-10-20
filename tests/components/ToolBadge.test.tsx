import { render, within } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { ToolBadge } from "@/components/tools/ToolBadge";
import { INSTALL_CATALOG } from "@/config/installCatalog";

describe("agent brand marks", () => {
  it.each(INSTALL_CATALOG)("uses a bundled logo for $label", (entry) => {
    const { container } = render(<ToolBadge toolId={entry.toolId} />);
    const logo = within(container).getByRole("img");
    expect(logo).toHaveAttribute("src");
    expect(logo.getAttribute("src")).not.toMatch(/^https?:\/\//);
    expect(logo.getAttribute("alt")).toBeTruthy();
  });

  it("keeps product identities distinct for Codex/ChatGPT and DevEco Code/CLI", () => {
    const logos = ["codex", "chatgpt", "deveco-code", "deveco-cli"].map(
      (toolId) => {
        const { container } = render(<ToolBadge toolId={toolId} />);
        return within(container).getByRole("img").getAttribute("src");
      },
    );
    expect(logos[0]).not.toBe(logos[1]);
    expect(logos[2]).not.toBe(logos[3]);
  });
});
