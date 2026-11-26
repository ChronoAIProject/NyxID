import { createContext, useContext, useMemo } from "react";
import { floatingScrollIndicatorProps } from "./workspaceTrailingInset";

/**
 * On the Duo (folded or unfolded) in rail mode, readable and interactive
 * content keeps clear of the floating rail: the shared content containers
 * (WorkspacePage, ScreenContainer) add this to their trailing padding,
 * animated by
 * WorkspaceRailMotionContext. The page background still runs to the edge
 * underneath. Zero in full screen (content runs to the 12pt margin),
 * elsewhere and inside sheets (they cover the rail).
 */
export const WorkspaceFloatingInsetContext = createContext(0);

/**
 * The Duo's rail, where it exists (folded or unfolded): the rail-mode inset,
 * whether full screen is on, and its UI-thread progress
 * (0 rail mode, 1 full screen) that animates the containers' inset and the
 * grouped capsule together.
 */
export type RailMotion = {
  inset: number;
  fullScreen: boolean;
  progress: { value: number };
};
export const WorkspaceRailMotionContext = createContext<RailMotion | null>(null);

export function useFloatingTrailingInset() {
  return useContext(WorkspaceFloatingInsetContext);
}

/**
 * Every vertical scroll view on a workspace page spreads these props
 * (`{...railIndicator}`): rail mode keeps its indicator between the content
 * and the rail; full screen puts it at the window edge. Empty where there
 * is no rail.
 */
export function useRailScrollIndicator() {
  const rail = useContext(WorkspaceRailMotionContext);
  const inset = rail?.inset ?? 0;
  const fullScreen = rail?.fullScreen ?? false;
  return useMemo(
    () => floatingScrollIndicatorProps(inset, fullScreen),
    [inset, fullScreen],
  );
}
