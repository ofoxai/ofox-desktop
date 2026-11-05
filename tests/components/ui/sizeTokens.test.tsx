import { describe, expect, it } from "vitest";
import { createRequire } from "node:module";
import { buttonVariants } from "@/components/ui/button";
import { menuTriggerClassName } from "@/components/ui/menu-select";

// The type sizes and control heights are variables a page can set for its
// own subtree (the wide assistant page does); unset, they are today's.

const require = createRequire(import.meta.url);
const config = require("../../../tailwind.config.cjs");

describe("size tokens", () => {
  it("keeps today's sizes when no variable is set", () => {
    expect(config.theme.extend.fontSize).toMatchObject({
      ui: "var(--fs-ui, 13px)",
      nav: "var(--fs-nav, 12.5px)",
      ctl: "var(--fs-ctl, 12px)",
      sub: "var(--fs-sub, 11.5px)",
      note: "var(--fs-note, 11px)",
      micro: "var(--fs-micro, 10.5px)",
      title: "var(--fs-title, 15px)",
    });
  });

  it("sizes an icon button and a menu trigger by their variables", () => {
    expect(buttonVariants({ variant: "icon" })).toContain(
      "h-[var(--icon-btn,26px)]",
    );
    expect(buttonVariants({ variant: "icon" })).toContain(
      "w-[var(--icon-btn,26px)]",
    );
    expect(menuTriggerClassName).toContain("h-[var(--ctl-h,26px)]");
  });
});
