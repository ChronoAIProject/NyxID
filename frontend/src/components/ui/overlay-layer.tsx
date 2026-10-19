import type { ReactNode } from "react";
import { OverlayLayerContext } from "@/lib/overlay-layer";

/** Each floating surface owns a layer for all its nested portals. */
export function OverlayLayer({
  layer,
  children,
}: {
  readonly layer: number;
  readonly children: ReactNode;
}) {
  return (
    <OverlayLayerContext.Provider value={layer}>
      {children}
    </OverlayLayerContext.Provider>
  );
}
