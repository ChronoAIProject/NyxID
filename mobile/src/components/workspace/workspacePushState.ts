export const SIDEBAR_PUSH_RATIO = 0.77;
export const SIDEBAR_EDGE_WIDTH = 24;
export const SIDEBAR_SETTLE_THRESHOLD = 0.45;

export function sidebarPushWidth(width: number) {
  "worklet";
  return Math.round(Math.max(0, width) * SIDEBAR_PUSH_RATIO);
}

export function clampSidebarOffset(offset: number, width: number) {
  "worklet";
  return Math.max(0, Math.min(width, offset));
}

/** Project a short flick while requiring deliberate travel for a slow drag. */
export function settleSidebarOpen(
  offset: number,
  velocity: number,
  width: number,
) {
  "worklet";
  if (width <= 0) return false;
  const projected = clampSidebarOffset(offset + velocity * 0.15, width);
  return projected >= width * SIDEBAR_SETTLE_THRESHOLD;
}

/** A sidebar root swap travels only with the slab; nested pages still push. */
export function workspaceContentAnimation(
  reducedMotion: boolean,
  sidebarPresented: boolean,
) {
  return reducedMotion || sidebarPresented
    ? ("none" as const)
    : ("slide_from_right" as const);
}
