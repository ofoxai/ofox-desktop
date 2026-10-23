/** Straight stacked areas over exact points. Null splits the path; zero stays zero. */
export function trendAreas(
  values: (number | null)[],
  lower: (number | null)[],
  max: number,
) {
  const segments: number[][] = [];
  let segment: number[] = [];
  values.forEach((value, index) => {
    if (value == null) {
      if (segment.length) segments.push(segment);
      segment = [];
    } else segment.push(index);
  });
  if (segment.length) segments.push(segment);
  const x = (index: number) => (index / Math.max(1, values.length - 1)) * 1000;
  const y = (value: number) => 200 - (value / Math.max(1, max)) * 200;
  const point = (index: number, value: number) => `${x(index)},${y(value)}`;
  return segments.map((indices) => {
    const top = indices.map((i) => point(i, values[i]!));
    const bottom = indices.map((i) => point(i, lower[i] ?? 0));
    return {
      point:
        indices.length === 1
          ? { x: x(indices[0]), y: y(values[indices[0]]!) }
          : null,
      line: `M${top.join(" L")}`,
      upper: `M${top.join(" L")} L${[...bottom].reverse().join(" L")} Z`,
      lower: `M${bottom.join(" L")} L${x(indices.at(-1)!)},200 L${x(indices[0])},200 Z`,
    };
  });
}
