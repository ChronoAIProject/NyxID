// Pure style math (no React), so tests load it unmocked.
type Style = Record<string, unknown>;

function flatten(style: unknown, into: Style = {}): Style {
  if (Array.isArray(style)) for (const part of style) flatten(part, into);
  else if (style && typeof style === "object") Object.assign(into, style);
  return into;
}

/**
 * A container's own resolved trailing padding; the rail inset animates on top
 * of it (RailInsetView).
 */
export function ownTrailingPadding(style: unknown): number {
  const flat = flatten(style);
  return (
    (flat.paddingRight as number | undefined) ??
    (flat.paddingEnd as number | undefined) ??
    (flat.paddingHorizontal as number | undefined) ??
    (flat.padding as number | undefined) ??
    0
  );
}

/**
 * Where a scroll view's vertical indicator sits relative to the floating
 * rail. iOS draws it ~3pt wide, ~2.8pt in from the scroll view's trailing
 * edge (measured on the Duo simulator).
 */
export const IOS_SCROLL_INDICATOR_WIDTH = 3;
export const IOS_SCROLL_INDICATOR_EDGE_GAP = 2.8;
/**
 * Scroll views inside a rail-inset container end 4pt under the glass (the
 * inset keeps a 12pt content margin plus an 8pt gap). Moving the indicator in
 * by this much centres it in that gap, between the content and the glass.
 */
export const SCROLL_INDICATOR_CLEARANCE = 4;

/**
 * Props every vertical workspace scroller spreads (it sits inside a rail-inset
 * container). Rail mode: explicit iOS indicator insets that keep the
 * indicator between the content and the rail, with UIKit's automatic
 * safe-area adjustment off (left on, it adds the window's 83pt right safe
 * area on top: measured, the indicator landed over the text). Full screen:
 * the indicator at the window edge; content keeps its natural top (no extra
 * top inset, so a scroller never rests below where its content starts) and
 * scrolls under the floating toggle like under a status bar. Empty without a
 * rail: native placement everywhere else.
 */
export function floatingScrollIndicatorProps(
  inset: number,
  fullScreen = false,
): {
  scrollIndicatorInsets?: { right: number };
  automaticallyAdjustsScrollIndicatorInsets?: boolean;
} {
  if (!inset) return {};
  if (fullScreen)
    return {
      scrollIndicatorInsets: { right: 0 },
      automaticallyAdjustsScrollIndicatorInsets: false,
    };
  return {
    scrollIndicatorInsets: { right: SCROLL_INDICATOR_CLEARANCE },
    automaticallyAdjustsScrollIndicatorInsets: false,
  };
}
