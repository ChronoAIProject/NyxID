// Pure geometry (no React), so tests load it unmocked.
/*
 * The rail toggle's icons: Lucide Maximize2/Minimize2's two bold arrows,
 * turned horizontal. Lucide has no such pair (fold/unfold-horizontal add a
 * dashed centre line), so they are drawn here on Lucide's grid: 24×24,
 * stroke width 2, round caps and joins, no fill, themed colour. They span
 * Lucide's full x 2–22; each arrowhead is a chevron with ±5 legs (close to
 * Maximize2's 6-unit corner heads) and the two arrows leave a gap in the
 * centre. The rail draws them at its shortcut icons' 20pt.
 */
export const ARROW_ICON_GRID = 24;
export const ARROW_ICON_STROKE_WIDTH = 2;

type Point = readonly [number, number];
/** A shaft and its chevron head; `head[1]` is the arrow's tip. */
export type HorizontalArrow = {
  shaft: readonly [Point, Point];
  head: readonly [Point, Point, Point];
};

/** Expand (enter full screen): ←  → , each pointing outward. */
export const EXPAND_HORIZONTAL_ARROWS: readonly HorizontalArrow[] = [
  { shaft: [[9, 12], [2, 12]], head: [[7, 7], [2, 12], [7, 17]] },
  { shaft: [[15, 12], [22, 12]], head: [[17, 7], [22, 12], [17, 17]] },
];

/** Restore (leave full screen): →  ← , each pointing in at the centre. */
export const RESTORE_HORIZONTAL_ARROWS: readonly HorizontalArrow[] = [
  { shaft: [[2, 12], [9, 12]], head: [[4, 7], [9, 12], [4, 17]] },
  { shaft: [[22, 12], [15, 12]], head: [[20, 7], [15, 12], [20, 17]] },
];

const point = ([x, y]: Point) => `${x} ${y}`;
export function horizontalArrowPath(arrow: HorizontalArrow) {
  const [from, to] = arrow.shaft;
  const [back, tip, other] = arrow.head;
  return `M${point(from)}L${point(to)}M${point(back)}L${point(tip)}L${point(other)}`;
}
