import { useEffect, useState } from "react";
import { ActivityIndicator, AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { z } from "zod";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useTheme } from "../../theme/ThemeContext";

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

/**
 * Pull to refresh shows its spinner only for the pull itself, so the
 * background polling of these pages never flashes it.
 */
export function usePullRefresh(refetch: () => Promise<unknown>) {
  const [refreshing, setRefreshing] = useState(false);
  return {
    refreshing,
    onRefresh: () => {
      setRefreshing(true);
      void refetch().finally(() => setRefreshing(false));
    },
  };
}

export function AutomationLoading() {
  const { colors } = useTheme();
  return (
    <ActivityIndicator
      accessibilityLabel="Loading automations"
      color={colors.primary}
      style={{ padding: 24 }}
    />
  );
}
