import { getAuthEpoch } from "../auth/authEpoch";
import { AccessibilityInfo, InteractionManager } from "react-native";
import { createWorkspaceFocusState } from "./workspaceFocusState";

export const workspaceFocus = createWorkspaceFocusState(
  (target) => AccessibilityInfo.setAccessibilityFocus(target),
  (callback) => {
    InteractionManager.runAfterInteractions(() =>
      requestAnimationFrame(callback),
    );
  },
  getAuthEpoch,
);
