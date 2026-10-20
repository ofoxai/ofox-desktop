import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import { ListCard, ListFold, ListHead, ListRow } from "@/components/ui/list";

describe("list", () => {
  it("draws magpie's list: a 12px card whose rows are 48px and split by hairlines", () => {
    render(
      <ListCard data-testid="card">
        <ListHead data-testid="head">2 个会话</ListHead>
        <ListRow data-testid="row">one</ListRow>
        <ListRow>two</ListRow>
      </ListCard>,
    );
    expect(screen.getByTestId("card")).toHaveClass(
      "overflow-hidden",
      "rounded-list",
      "border",
      "border-line",
      "bg-card",
      "divide-y",
      "divide-line-2",
    );
    expect(screen.getByTestId("head")).toHaveClass(
      "min-h-[34px]",
      "text-sub",
      "text-ink-muted",
    );
    expect(screen.getByTestId("row")).toHaveClass(
      "flex",
      "min-h-12",
      "gap-3",
      "py-[7px]",
      "pl-[14px]",
      "pr-3",
    );
  });

  it("folds with a disclosure button whose chevron turns a quarter when open", async () => {
    const user = userEvent.setup();
    function Fold() {
      const [open, setOpen] = useState(false);
      return (
        <>
          <ListFold open={open} onOpenChange={setOpen} note="3">
            未安装工具（3）
          </ListFold>
          {open && <div>folded rows</div>}
        </>
      );
    }
    render(<Fold />);
    const fold = screen.getByRole("button", { name: /未安装工具（3）/ });
    expect(fold).toHaveAttribute("aria-expanded", "false");
    expect(fold.querySelector("svg.lucide-chevron-right")).toHaveClass(
      "group-aria-expanded:rotate-90",
    );
    expect(screen.queryByText("folded rows")).toBeNull();

    await user.click(fold);
    expect(fold).toHaveAttribute("aria-expanded", "true");
    expect(screen.getByText("folded rows")).toBeInTheDocument();
  });
});
