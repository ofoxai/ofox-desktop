import fs from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";

interface WindowConfig {
  label: string;
  titleBarStyle?: string;
  trafficLightPosition?: { x: number; y: number };
}

const config = JSON.parse(
  fs.readFileSync(
    path.resolve(__dirname, "../../src-tauri/tauri.conf.json"),
    "utf8",
  ),
) as { app: { windows: WindowConfig[] } };

const main = config.app.windows.find((window) => window.label === "main");

describe("macOS main window", () => {
  it("draws its traffic lights over the app's 46px header", () => {
    expect(main?.titleBarStyle).toBe("Overlay");
  });

  // 14,17 centres the 12px buttons on the header's middle (23px) and keeps
  // them inside the 78px the header leaves them.
  it("puts the traffic lights level with the header's tabs", () => {
    expect(main?.trafficLightPosition).toEqual({ x: 14, y: 17 });
  });
});
