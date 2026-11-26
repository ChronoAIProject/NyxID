import { useState } from "react";
import type { LayoutChangeEvent } from "react-native";

/** Shared signed-in header geometry, measured below the window safe area. */
export const WORKSPACE_HEADER_ROW_HEIGHT = 60;
export const WORKSPACE_HEADER_CONTROL_SIZE = 44;
// Page titles (header bar and in-page large title) grow with Dynamic Type up
// to this multiple; the header bar always carries the full page name.
export const WORKSPACE_TITLE_MAX_FONT_SCALE = 1.25;
// The in-page large title wraps between words, then ends in an ellipsis.
export const WORKSPACE_TITLE_MAX_LINES = 2;
// Every slab bar (narrow, or beside the persistent sidebar) shares the
// transcript's and composer's 12pt side margins and spaces its floating glass
// controls 8pt apart.
export const FLOATING_HEADER_MARGIN = 12;
export const FLOATING_HEADER_GAP = 8;
export const WORKSPACE_HEADER_ROW = {
  height: WORKSPACE_HEADER_ROW_HEIGHT,
  flexDirection: "row" as const,
  alignItems: "center" as const,
};

/** Equal control columns keep the title centered on the full slab, even with asymmetric actions. */
export function useWorkspaceHeaderSides() {
  const [left, setLeft] = useState(WORKSPACE_HEADER_CONTROL_SIZE);
  const [right, setRight] = useState(WORKSPACE_HEADER_CONTROL_SIZE);
  return {
    width: Math.max(WORKSPACE_HEADER_CONTROL_SIZE, left, right),
    leftLayout: (event: LayoutChangeEvent) =>
      setLeft(event.nativeEvent.layout.width),
    rightLayout: (event: LayoutChangeEvent) =>
      setRight(event.nativeEvent.layout.width),
  };
}
