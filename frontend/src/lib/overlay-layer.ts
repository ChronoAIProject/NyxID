import { createContext, useContext } from "react";

// Context follows portals; CSS inheritance does not.
export const OverlayLayerContext = createContext(40);
export const ASSISTANT_OVERLAY_BASE = 80;
// Credits content 120, backdrop 119, descendants 130: above panel grandchildren 110.
export const ASSISTANT_CREDITS_OVERLAY_BASE = 110;
export function useOverlayLayer() {
  return useContext(OverlayLayerContext) + 10;
}
