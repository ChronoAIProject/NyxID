import { useLocalizedState } from "../../i18n/useLocalizedState";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useEffect, useRef, useState } from "react";
import { Alert, AppState } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import * as WebBrowser from "expo-web-browser";
import { z } from "zod";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { MachineReviewRequired } from "./machineContracts";
import { ApiError } from "../../lib/api/ApiError";
import { getFrontendUrl } from "../../lib/env";

export const machineKeys = (epoch: number) =>
  ["workspace", "machines", epoch] as const;
export function machineError(cause: unknown) {
  if (cause instanceof ApiError)
    return (cause.reason ? translateApp("{{message}} (code {{code}}, {{reason}})", { message: cause.message, code: cause.errorCode, reason: cause.reason }) : translateApp("{{message}} (code {{code}})", { message: cause.message, code: cause.errorCode}));
  if (cause instanceof z.ZodError)
    return translateApp(
      "The server returned unsupported machine data. Refresh or open Machines on the web.",
    );
  return cause instanceof Error
    ? cause.message
    : translateApp("Could not complete this machine action.");
}
export async function openMachineWeb(path: string) {
  await WebBrowser.openBrowserAsync(
    `${getFrontendUrl().replace(/\/$/, "")}${path}`,
  );
}

export function useMachineRuntime() {
  useAppLanguage();
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  const epoch = getAuthEpoch();
  const state = useRef({ mounted: true, focused, isConnected });
  state.current.focused = focused;
  state.current.isConnected = isConnected;
  useEffect(() => {
    state.current.mounted = true;
    const subscription = AppState.addEventListener("change", (value) =>
      setForeground(value === "active"),
    );
    return () => {
      state.current.mounted = false;
      subscription.remove();
    };
  }, []);
  return {
    epoch,
    active: focused && foreground && isConnected,
    online: isConnected,
    current: () => state.current.mounted && getAuthEpoch() === epoch,
    interactive: () =>
      state.current.mounted &&
      getAuthEpoch() === epoch &&
      state.current.focused &&
      AppState.currentState === "active" &&
      state.current.isConnected,
  };
}

export function useMachineWrite() {
  useAppLanguage();
  const runtime = useMachineRuntime();
  const lock = useRef(false);
  const needsReview = useRef(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useLocalizedState<string>();
  const [uncertain, setUncertain] = useState(false);
  async function run(action: (current: () => boolean) => Promise<void>) {
    if (lock.current || needsReview.current || !runtime.interactive()) return;
    lock.current = true;
    setBusy(true);
    setError(undefined);
    try {
      await action(() => runtime.current() && runtime.interactive());
    } catch (cause) {
      if (runtime.current()) {
        const unknown =
          !(cause instanceof MachineReviewRequired) &&
          (!(cause instanceof ApiError) || cause.statusCode >= 500);
        needsReview.current = unknown;
        setUncertain(unknown);
        setError(
          () => ((unknown ? translateApp("{{machineError}} The action may have reached the server. Refresh and review status before taking another action.", { machineError: machineError(cause) }) : machineError(cause))),
        );
      }
    } finally {
      lock.current = false;
      if (runtime.current()) setBusy(false);
    }
  }
  return {
    ...runtime,
    busy,
    error,
    uncertain,
    run,
    reviewed: () => {
      needsReview.current = false;
      setUncertain(false);
      setError(undefined);
    },
  };
}

export function confirmDiscard(dirty: boolean, leave: () => void) {
  if (!dirty) {
    leave();
    return;
  }
  Alert.alert(
    translateApp("Discard machine changes?"),
    translateApp("Your unsaved changes will be lost."),
    [
      { text: translateApp("Keep editing"), style: "cancel" },
      { text: translateApp("Discard"), style: "destructive", onPress: leave },
    ],
  );
}
