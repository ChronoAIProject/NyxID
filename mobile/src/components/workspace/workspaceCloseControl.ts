import type { ViewStyle } from "react-native";

/** Shared by the navigation menu and management drawer header. */
export const WORKSPACE_CLOSE_ICON_SIZE = 22;
export function workspaceCloseControlStyle(backgroundColor: string): ViewStyle {
  return {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
    backgroundColor,
  };
}
