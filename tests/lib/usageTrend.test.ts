import { expect, it } from "vitest";
import { trendAreas } from "@/lib/usageTrend";
it("breaks at missing values, preserves real zero and uses exact straight segments", () => {
  const areas = trendAreas([10, 0, null, 20, 10], [5, 0, null, 10, 5], 20);
  expect(areas).toHaveLength(2);
  expect(areas[0].line).toBe("M0,100 L250,200");
  expect(areas[1].line).toBe("M750,0 L1000,100");
  expect(areas[0].lower).toContain("0,150");
  expect(areas.every((area) => !/C|NaN|Infinity/.test(area.upper))).toBe(true);
});

it("keeps isolated known values visible between missing intervals", () => {
  const [area] = trendAreas([null, 10, null], [null, 4, null], 20);
  expect(area.point).toEqual({ x: 500, y: 100 });
});
