import { createContext, useContext } from "react";
import type { NavigationState, PartialState } from "@react-navigation/native";

export type WorkspaceAccessibility = {
  sheetOpen: boolean;
  primaryKey?: string;
};
export const WorkspaceAccessibilityContext =
  createContext<WorkspaceAccessibility>({ sheetOpen: false });
export const useWorkspaceAccessibility = () =>
  useContext(WorkspaceAccessibilityContext);

export function workspaceAccessibilityState(
  state: NavigationState | PartialState<NavigationState> | undefined,
): WorkspaceAccessibility {
  const index = state?.index ?? 0;
  const route = state?.routes[index];
  const sheetOpen = route?.name === "WorkspaceSheet";
  const primary = sheetOpen ? state?.routes[index - 1] : route;
  return { sheetOpen, primaryKey: primary?.key };
}
