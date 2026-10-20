import { cn } from "@/lib/utils";

const HEIGHT = 52;
const PAD = 3;

/** "x,y" pairs on a 100×52 box, the lowest point at the bottom. */
function linePoints(points: ReadonlyArray<number>): string {
  const min = Math.min(...points);
  const span = Math.max(...points) - min || 1;
  const last = Math.max(points.length - 1, 1);
  const y = (value: number) =>
    PAD + (1 - (value - min) / span) * (HEIGHT - 2 * PAD);
  const pairs = points.map(
    (value, index) =>
      `${Math.round((10000 * index) / last) / 100},${Math.round(y(value) * 100) / 100}`,
  );
  // A single point still reads as a level line across the box.
  if (points.length === 1)
    pairs.push(`100,${Math.round(y(points[0]) * 100) / 100}`);
  return pairs.join(" ");
}

/** A 52px trend line in the brand colour, stretched to its width. */
export function Sparkline({
  points,
  className,
  "aria-label": ariaLabel,
}: {
  points: ReadonlyArray<number>;
  className?: string;
  "aria-label"?: string;
}) {
  return (
    <svg
      viewBox={`0 0 100 ${HEIGHT}`}
      preserveAspectRatio="none"
      role={ariaLabel ? "img" : undefined}
      aria-label={ariaLabel}
      aria-hidden={ariaLabel ? undefined : true}
      className={cn("block h-[52px] w-full text-brand", className)}
    >
      {points.length > 0 && (
        <polyline
          points={linePoints(points)}
          fill="none"
          stroke="currentColor"
          strokeWidth={1.5}
          strokeLinejoin="round"
          strokeLinecap="round"
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}
