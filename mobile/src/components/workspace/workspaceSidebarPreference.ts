import { useStoredFlag } from "./workspaceDevicePreference";

/**
 * Per device: whether the persistent (wide-window) sidebar is open. Closed by
 * default, so content takes the full width; Menu opens it and the person's
 * later choice is kept. Independent of the folded Duo's quick-rail full screen.
 */
export const SIDEBAR_OPEN_KEY = "nyxid.workspace.sidebar_open";

export function useSidebarOpen() {
  return useStoredFlag(SIDEBAR_OPEN_KEY);
}
