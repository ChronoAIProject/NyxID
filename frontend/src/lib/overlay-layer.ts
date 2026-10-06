import { createContext, useContext } from "react";

// Context follows portals; CSS inheritance does not.
export const OverlayLayerContext = createContext(40);
export const ASSISTANT_OVERLAY_BASE = 80;
export function useOverlayLayer() {
  return useContext(OverlayLayerContext) + 10;
}
