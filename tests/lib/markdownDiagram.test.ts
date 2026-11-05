import { describe, expect, it } from "vitest";
import {
  cleanDiagram,
  diagramImage,
  diagramSource,
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
  it("turns a tag that is not plain formatting into text", () => {
    expect(
      diagramSource(
        'flowchart LR\n  X["<img src=x onerror=go()>"] --> A["<b>bold</b> <b/onclick=go()>x</b>"]',
      ),
    ).toBe(
      'flowchart LR\n  X["‹img src=x onerror=go()>"] --> A["<b>bold</b> ‹b/onclick=go()>x</b>"]',
    );
  });

  it("cannot configure Mermaid, however the configuration is written", () => {
    const configured = [
      "---  \nconfig:\n  themeCSS: 'body{display:none}'\n---  \nflowchart LR\n  A --> B",
      "%%{init: {'themeCSS': '.n{position:fixed}'}}%%\nflowchart LR\n  A --> B",
    ];
    for (const source of configured)
      expect(diagramSource(source)?.trim()).toBe("flowchart LR\n  A --> B");
    // An unclosed directive takes the rest with it, as it does in Mermaid.
    expect(
      diagramSource("flowchart LR\n  A --> B\n%%{init: {'theme': 'x'}"),
    ).toBe("flowchart LR\n  A --> B\n");
  });

  it("is not drawn when it reaches for something", () => {
    for (const source of [
      "flowchart LR\n  A --> B; style B fill:url(//x.test/p)",
      "flowchart LR\n  A --> B\n  style B fill:u\\72l(//x.test/p)",
      "flowchart LR\n  A --> B\n  classDef c background:image-set('x.png' 1x)",
      'flowchart LR\n  A@{ img: "https://x.test/a.png", label: "a" }',
      "flowchart LR\n  A --> B\n  style B font:@import x",
    ])
      expect(diagramSource(source)).toBeNull();
  });

  it("keeps Mermaid's own arrows, annotations, styles and line breaks", () => {
    const source =
      'classDiagram\n  class Shape{\n    <<interface>>\n  }\n  Shape <|-- Circle\nflowchart LR\n  A <--> B\n  B -->|a<br>b| C\n  style C fill:#f9f,stroke:#333\n  D["line\\nnext"]';
    expect(diagramSource(source)).toBe(source);
  });
});
