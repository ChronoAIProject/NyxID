import {
  loadStoredFlag,
  saveStoredFlag,
  useStoredFlag,
} from "./workspaceDevicePreference";

/** Per device: full-screen mode hides the folded Duo's quick rail. */
export const QUICK_RAIL_HIDDEN_KEY = "nyxid.workspace.quick_rail_hidden";

export function loadQuickRailHidden(): Promise<boolean> {
  return loadStoredFlag(QUICK_RAIL_HIDDEN_KEY);
}

export function saveQuickRailHidden(hidden: boolean) {
  return saveStoredFlag(QUICK_RAIL_HIDDEN_KEY, hidden);
}

/** Default shown; a choice made before the saved value loads wins. */
export function useQuickRailHidden() {
  return useStoredFlag(QUICK_RAIL_HIDDEN_KEY);
}
