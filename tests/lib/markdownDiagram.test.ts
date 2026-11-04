import { describe, expect, it } from "vitest";
import {
  cleanDiagram,
  diagramImage,
  quietDiagramSource,
} from "@/lib/markdownDiagram";

const svg = (inner: string, attributes = "") =>
  `<svg xmlns="http://www.w3.org/2000/svg" id="d" ${attributes}>${inner}</svg>`;

describe("a diagram's SVG", () => {
  it("keeps its shapes, text, style and its own markers", () => {
    const out = cleanDiagram(
      svg(
        '<style>#d .edge{stroke:#333}</style><g><rect width="10" height="10"></rect><text>用户提问</text><path marker-end="url(#d_arrow)" d="M0 0L1 1"></path></g>',
      ),
    );
    expect(out).toContain("<rect");
    expect(out).toContain("用户提问");
    expect(out).toContain('marker-end="url(#d_arrow)"');
    expect(out).toContain("stroke:#333");
  });

  it("loses images, link targets and script", () => {
    const out = cleanDiagram(
      svg(
        '<image href="https://x.test/i.png"></image><foreignObject><div><img src="https://x.test/t.png"></div></foreignObject><a href="https://x.test"><text>Open</text></a><script>alert(1)</script><g onclick="alert(2)"></g>',
      ),
    );
    expect(out).toContain("Open");
    expect(out).not.toContain("x.test");
    expect(out).not.toContain("alert");
  });

  it("becomes an image of well-formed SVG, as wide as it was drawn", () => {
    const image = diagramImage(
      svg(
        "<foreignObject><div>a<br>b</div></foreignObject>",
        'viewBox="0 0 320 120"',
      ),
    );
    expect(image?.width).toBe(320);
    expect(image?.src.startsWith("data:image/svg+xml;charset=utf-8,")).toBe(
      true,
    );
    const xml = decodeURIComponent(image!.src.split(",")[1]);
    const parsed = new DOMParser().parseFromString(xml, "image/svg+xml");
    expect(parsed.querySelector("parsererror")).toBeNull();
    expect(xml).toContain("<br />");
  });

  it("is no image when there is no SVG", () => {
    expect(diagramImage("<p>none</p>")).toBeNull();
  });
});

describe("a diagram's text before it is drawn", () => {
  it("loses markup that loads something and every attribute", () => {
    expect(
      quietDiagramSource(
        'flowchart LR\n  X["<img src=x onerror=go()>"] --> A["<b style=\'color:red\'>bold</b>"]',
      ),
    ).toBe('flowchart LR\n  X[" "] --> A["<b>bold</b>"]');
  });

  it("cannot configure Mermaid or style with a reference", () => {
    expect(
      quietDiagramSource(
        [
          "---",
          "config:",
          "  themeCSS: 'body{display:none}'",
          "---",
          "%%{init: {'themeCSS': '.n{position:fixed}'}}%%",
          "flowchart LR",
          "  A --> B",
          "  style A fill:#f9f,stroke:#333",
          "  style B fill:u\\72l(//x.test/p)",
          "  classDef c background:url(//x.test/q)",
        ].join("\n"),
      ),
    ).toBe("flowchart LR\n  A --> B\n  style A fill:#f9f,stroke:#333");
  });

  it("keeps Mermaid's own arrows, annotations and line breaks", () => {
    const source =
      "classDiagram\n  class Shape{\n    <<interface>>\n  }\n  Shape <|-- Circle\nflowchart LR\n  A <--> B\n  B -->|a<br>b| C";
    expect(quietDiagramSource(source)).toBe(source);
  });
});
