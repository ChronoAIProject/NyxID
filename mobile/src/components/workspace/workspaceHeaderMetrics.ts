import { useState } from "react";
import type { LayoutChangeEvent } from "react-native";

/** Shared signed-in header geometry, measured below the window safe area. */
export const WORKSPACE_HEADER_ROW_HEIGHT = 60;
export const WORKSPACE_HEADER_CONTROL_SIZE = 44;
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
