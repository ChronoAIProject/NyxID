import { createContext } from "react";

export const WorkspaceCornerSensorContext = createContext(false);
/**
 * The Duo's quick rail (folded or unfolded) is available, in rail mode or
 * full screen: it carries New chat, Notifications, Scan QR, Recent chats and Settings,
 * and a chat's bar controls (so chats have no top bar there).
 */
export const WorkspaceQuickRailContext = createContext(false);
/**
 * A workspace sheet's header band, relative to where its header begins: the
 * row sits flush under the handle beside a corner status column, and the
 * full-width body starts below the cluster.
 */
export const WorkspaceSheetHeaderBandContext = createContext({
  paddingTop: 0,
  paddingBottom: 0,
});
/**
 * The window a workspace sheet opens in shows the Duo's quick rail (folded or
 * unfolded). Sheets rewrite their own safe-area insets, so they read this
 * instead of measuring; Settings offers Quick access only where it is true.
 */
export const WorkspaceRailWindowContext = createContext(false);
