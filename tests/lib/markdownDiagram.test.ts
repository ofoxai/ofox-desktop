import { describe, expect, it } from "vitest";
import { cleanDiagram, quietDiagramSource } from "@/lib/markdownDiagram";

const svg = (inner: string) =>
  `<svg xmlns="http://www.w3.org/2000/svg" id="d">${inner}</svg>`;

describe("a diagram as it goes into the page", () => {
  it("keeps its shapes, text and its own markers", () => {
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

  it("loads nothing: no image, no remote style, no import", () => {
    const out = cleanDiagram(
      svg(
        '<style>@import "https://x.test/a.css"; #d .n{fill:url(https://x.test/p.png)}</style><image href="https://x.test/i.png"></image><foreignObject><div><img src="https://x.test/t.png"></div></foreignObject><rect style="fill:url(https://x.test/q.png)"></rect>',
      ),
    );
    expect(out).not.toContain("x.test");
  });

  it("opens nothing: a link loses its target, and no script survives", () => {
    const out = cleanDiagram(
      svg(
        '<a href="https://x.test"><text>Open</text></a><script>alert(1)</script><g onclick="alert(2)"></g>',
      ),
    );
    expect(out).toContain("Open");
    expect(out).not.toContain("x.test");
    expect(out).not.toContain("alert");
  });
});

describe("a diagram's text before it is drawn", () => {
  it("loses markup that loads something and every attribute", () => {
    expect(
      quietDiagramSource(
        'flowchart LR\n  X["<img src=x onerror=go()>"] --> A["<b style=\'background:url(https://x.test)\'>bold</b>"]',
      ),
    ).toBe('flowchart LR\n  X[" "] --> A["<b>bold</b>"]');
  });

  it("keeps Mermaid's own arrows, annotations and line breaks", () => {
    const source =
      "classDiagram\n  class Shape{\n    <<interface>>\n  }\n  Shape <|-- Circle\nflowchart LR\n  A <--> B\n  B -->|a<br>b| C";
    expect(quietDiagramSource(source)).toBe(source);
  });
});
