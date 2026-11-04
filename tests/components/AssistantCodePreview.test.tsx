import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import en from "@/i18n/locales/en.json";
import { SessionMarkdown } from "@/components/sessions/SessionMarkdown";
import {
  HTML_PREVIEW_POLICY,
  htmlHasScripts,
  htmlPreviewDocument,
} from "@/lib/htmlPreview";

const diagram = vi.hoisted(() => ({ draw: vi.fn() }));
vi.mock("@/lib/markdownDiagram", () => ({ drawDiagram: diagram.draw }));

beforeEach(async () => {
  i18n.addResourceBundle("en", "translation", en, true, true);
  await i18n.changeLanguage("en");
  diagram.draw.mockReset();
});

const mermaid = "```mermaid\nflowchart LR\n  A --> B\n```";
const html =
  "```html\n<!doctype html><h1>Hi</h1><script>alert(1)</script>\n```";

describe("a Mermaid block in a reply", () => {
  it("is drawn as its diagram, and its source is a click away", async () => {
    diagram.draw.mockResolvedValue({
      src: "data:image/svg+xml;charset=utf-8,%3Csvg%3E%3C%2Fsvg%3E",
      width: 300,
    });
    render(<SessionMarkdown content={mermaid} previews />);
    expect(
      await screen.findByRole("img", { name: "Mermaid diagram" }),
    ).toBeInTheDocument();
    expect(diagram.draw).toHaveBeenCalledWith(
      "flowchart LR\n  A --> B",
      expect.any(Boolean),
    );
    await userEvent.click(screen.getByRole("button", { name: "Source" }));
    expect(screen.getByText(/flowchart LR/)).toBeInTheDocument();
    expect(screen.queryByRole("img", { name: "Mermaid diagram" })).toBeNull();
  });

  it("stays its code when it cannot be drawn", async () => {
    diagram.draw.mockResolvedValue(null);
    render(<SessionMarkdown content={mermaid} previews />);
    await vi.waitFor(() => expect(diagram.draw).toHaveBeenCalled());
    expect(screen.getByText(/flowchart LR/)).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Diagram" })).toBeNull();
  });

  it("is only code in a session transcript", async () => {
    render(<SessionMarkdown content={mermaid} />);
    await new Promise((resolve) => setTimeout(resolve, 300));
    expect(diagram.draw).not.toHaveBeenCalled();
    expect(screen.getByText(/flowchart LR/)).toBeInTheDocument();
  });
});

describe("an HTML block in a reply", () => {
  it("starts as code and previews in a frame that loads nothing", async () => {
    render(<SessionMarkdown content={html} previews />);
    expect(screen.queryByTitle("HTML preview")).toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Preview" }));
    const frame = screen.getByTitle("HTML preview");
    expect(frame).toHaveAttribute("sandbox", "");
    expect(frame.getAttribute("srcdoc")).toBe(
      htmlPreviewDocument(
        "<!doctype html><h1>Hi</h1><script>alert(1)</script>",
      ),
    );
    expect(
      screen.getByText(
        "The preview runs none of the page's scripts and loads nothing from the network.",
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Code" }));
    expect(screen.queryByTitle("HTML preview")).toBeNull();
  });

  it("offers no preview in a session transcript", () => {
    render(<SessionMarkdown content={html} />);
    expect(screen.queryByRole("button", { name: "Preview" })).toBeNull();
  });
});

describe("the preview document", () => {
  it("puts its policy before the page and leaves out a refresh", () => {
    const page = htmlPreviewDocument(
      '<!DOCTYPE html><html><head><meta http-equiv="refresh" content="0;url=https://x.test"></head><body>ok</body></html>',
    );
    expect(page).toBe(
      `<!doctype html><meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_POLICY}"><html><head></head><body>ok</body></html>`,
    );
    expect(HTML_PREVIEW_POLICY).toContain("default-src 'none'");
  });

  it("tells a page with script from one without", () => {
    expect(htmlHasScripts('<button onclick="go()">x</button>')).toBe(true);
    expect(htmlHasScripts("<p>plain</p>")).toBe(false);
  });
});
