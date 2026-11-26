import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import {
  ARROW_ICON_GRID,
  ARROW_ICON_STROKE_WIDTH,
  EXPAND_HORIZONTAL_ARROWS,
  RESTORE_HORIZONTAL_ARROWS,
  horizontalArrowPath,
  type HorizontalArrow,
} from "./horizontalArrowGeometry";

const xs = (arrow: HorizontalArrow) => [
  ...arrow.shaft.map(([x]) => x),
  ...arrow.head.map(([x]) => x),
];
/** -1: the arrow points left (its tip is its leftmost point); +1: right. */
const direction = (arrow: HorizontalArrow) =>
  Math.sign(arrow.head[1][0] - arrow.head[0][0]);

test("both toggle icons sit on Lucide's grid: 24×24, stroke 2, inside the stroke-safe area, mirror-symmetric", () => {
  assert.equal(ARROW_ICON_GRID, 24);
  assert.equal(ARROW_ICON_STROKE_WIDTH, 2, "Lucide's default stroke weight");
  for (const arrows of [EXPAND_HORIZONTAL_ARROWS, RESTORE_HORIZONTAL_ARROWS]) {
    assert.equal(arrows.length, 2, "two arrows");
    for (const arrow of arrows) {
      // Horizontal: a level shaft, a head symmetric about it, the tip at the
      // shaft's end.
      assert.ok(arrow.shaft.every(([, y]) => y === 12));
      assert.deepEqual(arrow.head[1], arrow.shaft[1], "the tip ends the shaft");
      assert.equal(arrow.head[0][0], arrow.head[2][0]);
      assert.equal(arrow.head[0][1] + arrow.head[2][1], 24);
      assert.equal(Math.abs(arrow.head[1][0] - arrow.head[0][0]), 5, "±5 chevron legs");
      assert.equal(Math.abs(arrow.head[0][1] - 12), 5);
      // Half the stroke stays inside the grid.
      for (const [x, y] of [...arrow.shaft, ...arrow.head]) {
        assert.ok(x >= 2 && x <= 22 && y >= 2 && y <= 22);
      }
    }
    // Lucide's full span (x 2–22), mirrored about the centre.
    const all = arrows.flatMap(xs);
    assert.deepEqual([Math.min(...all), Math.max(...all)], [2, 22]);
    const [left, right] = arrows;
    assert.deepEqual(
      xs(left!).map((x) => 24 - x).sort(),
      xs(right!).sort(),
    );
    // A small gap in the centre: nothing between 9 and 15, so 4 units stay
    // open after the round caps (3.3pt at the rail's 20pt size).
    assert.ok(all.every((x) => x <= 9 || x >= 15));
    assert.equal(15 - 9 - ARROW_ICON_STROKE_WIDTH, 4);
  }
});

test("expand points outward ( ←  → ); restore points inward ( →  ← )", () => {
  const [expandLeft, expandRight] = EXPAND_HORIZONTAL_ARROWS;
  // Expand: the left arrow points left with its tip at the icon's left edge,
  // the right arrow points right with its tip at the right edge.
  assert.equal(direction(expandLeft!), -1);
  assert.equal(direction(expandRight!), 1);
  assert.equal(expandLeft!.head[1][0], 2);
  assert.equal(expandRight!.head[1][0], 22);
  const [restoreLeft, restoreRight] = RESTORE_HORIZONTAL_ARROWS;
  // Restore: both point at the centre, tips facing across the gap.
  assert.equal(direction(restoreLeft!), 1);
  assert.equal(direction(restoreRight!), -1);
  assert.equal(restoreLeft!.head[1][0], 9);
  assert.equal(restoreRight!.head[1][0], 15);
  // Each shaft runs from its tail to the tip.
  for (const arrow of [expandLeft!, expandRight!, restoreLeft!, restoreRight!])
    assert.equal(
      Math.sign(arrow.shaft[1][0] - arrow.shaft[0][0]),
      direction(arrow),
    );
});

test("the icons render as Lucide-style strokes in the themed colour", () => {
  const h = hookHarness<{ type: unknown; props: Record<string, any> }>({
    "react/jsx-runtime": {
      jsx: (type: unknown, props: unknown) => ({ type, props }),
      jsxs: (type: unknown, props: unknown) => ({ type, props }),
    },
    "react-native-svg": { __esModule: true, default: "Svg", Path: "Path" },
  });
  const icons = h.load(
    fileURLToPath(new URL("./HorizontalArrowIcons.tsx", import.meta.url)),
  ) as any;
  for (const [name, arrows] of [
    ["ExpandHorizontalIcon", EXPAND_HORIZONTAL_ARROWS],
    ["RestoreHorizontalIcon", RESTORE_HORIZONTAL_ARROWS],
  ] as const) {
    h.mount(() => icons[name]({ size: 20, color: "#17171A" }));
    // The named icon renders the shared stroke icon with its arrows.
    const icon = h.value() as any;
    const svg = typeof icon.type === "function" ? icon.type(icon.props) : icon;
    assert.equal(svg.type, "Svg");
    assert.equal(svg.props.width, 20);
    assert.equal(svg.props.height, 20);
    assert.equal(svg.props.viewBox, "0 0 24 24");
    assert.equal(svg.props.fill, "none");
    assert.equal(svg.props.stroke, "#17171A");
    assert.equal(svg.props.strokeWidth, 2);
    assert.equal(svg.props.strokeLinecap, "round");
    assert.equal(svg.props.strokeLinejoin, "round");
    assert.deepEqual(
      svg.props.children.map((path: any) => path.props.d),
      arrows.map(horizontalArrowPath),
    );
    h.unmount();
  }
  assert.equal(
    horizontalArrowPath(EXPAND_HORIZONTAL_ARROWS[0]!),
    "M9 12L2 12M7 7L2 12L7 17",
  );
});
