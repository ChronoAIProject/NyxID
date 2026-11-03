import { useEffect, useState } from "react";
import { AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { z } from "zod";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { SkeletonRows } from "../../components/Skeleton";

export const AUTOMATIONS = ["workspace", "automations"] as const;
export const AUTOMATION_SETTINGS = [...AUTOMATIONS, "settings"] as const;
export function automationError(error: unknown): string {
  if (error instanceof z.ZodError)
    return error.issues[0]?.message ?? "Could not read automations.";
  return error instanceof Error
    ? error.message
    : "Could not load automations. Try again.";
}

/** Poll only while the page is visible and the app is active. */
export function useAutomationRuntime() {
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (value) =>
      setForeground(value === "active"),
    );
    return () => subscription.remove();
  }, []);
  return { active: focused && foreground, online: isConnected };
}

export { usePullRefresh } from "../../hooks/usePullRefresh";

/** First load of an automations page: rows in the shape they will take. */
export function AutomationLoading() {
  return <SkeletonRows rows={3} accessibilityLabel="Loading automations" />;
}
