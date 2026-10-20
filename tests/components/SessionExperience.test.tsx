import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import i18n from "i18next";
import zh from "@/i18n/locales/zh.json";
import { TranscriptPart } from "@/components/sessions/TranscriptPart";
import * as clipboard from "@/lib/clipboard";

import { settingsApi } from "@/lib/api/settings";

beforeEach(() => {
  vi.restoreAllMocks();
  i18n.addResourceBundle("zh", "translation", zh, true, true);
  i18n.addResourceBundle(
    "zh",
    "translation",
    { sessionExperience: zh.sessionExperience },
    true,
    true,
  );
});

describe("session reading", () => {
  it("renders readable Markdown while rejecting HTML, unsafe links and remote image loads", () => {
    const { container } = render(
      <TranscriptPart
        message={{
          role: "assistant",
          content:
            "## A useful answer\n\n**Read this**\n\n<script>alert(1)</script>\n\n![private](https://tracker.invalid/pixel)\n\n[unsafe](javascript:alert%281%29)",
        }}
      />,
    );
    expect(
      screen.getByRole("heading", { name: "A useful answer" }),
    ).toBeVisible();
    expect(container.querySelector("strong")).toHaveTextContent("Read this");
    expect(container.querySelector("script")).toBeNull();
    expect(container.querySelector("img")).toBeNull();
    expect(container.querySelector('a[href^="javascript:"]')).toBeNull();
  });

  it("copies a fenced code block without Markdown decoration and exposes message time", async () => {
    const copy = vi.spyOn(clipboard, "copyText").mockResolvedValue();
    const stamp = 1_760_000_000_000;
    const { container } = render(
      <TranscriptPart
        message={{
          role: "assistant",
          content: "```ts\nconst answer = 42;\n```",
          ts: stamp,
        }}
      />,
    );
    await userEvent.click(screen.getByRole("button", { name: "复制代码" }));
    expect(copy).toHaveBeenCalledWith("const answer = 42;");
    expect(container.querySelector("time")).toHaveAttribute(
      "datetime",
      new Date(stamp).toISOString(),
    );
  });

  it("keeps tool and thinking process content folded by default", () => {
    const { container } = render(
      <TranscriptPart
        message={{ role: "thinking", content: "private reasoning process" }}
      />,
    );
    expect(container.querySelector("details")).not.toHaveAttribute("open");
  });

  it("keeps literal HTML/XML questions readable without creating executable HTML", () => {
    const { container } = render(
      <TranscriptPart
        message={{
          role: "user",
          content:
            '<configuration key="example">How do I fix this?</configuration>',
        }}
      />,
    );
    expect(
      screen.getByText(
        '<configuration key="example">How do I fix this?</configuration>',
      ),
    ).toBeVisible();
    expect(container.querySelector("configuration")).toBeNull();
  });

  it("renders task lists without introducing native form controls into the reader", () => {
    const { container } = render(
      <TranscriptPart
        message={{
          role: "assistant",
          content: "- [x] Complete\n- [ ] Pending",
        }}
      />,
    );
    expect(screen.getByText("Complete")).toBeVisible();
    expect(container.querySelector("input")).toBeNull();
  });

  it("only offers links supported by the external opener and normalizes an HTTP URL before opening", async () => {
    const open = vi.spyOn(settingsApi, "openExternal").mockResolvedValue();
    render(
      <TranscriptPart
        message={{
          role: "assistant",
          content:
            "[Email](mailto:example@example.invalid) [Docs](HTTPS://EXAMPLE.INVALID/docs)",
        }}
      />,
    );
    expect(screen.queryByRole("link", { name: "Email" })).toBeNull();
    await userEvent.click(screen.getByRole("link", { name: "Docs" }));
    expect(open).toHaveBeenCalledWith("https://example.invalid/docs");
  });
});
