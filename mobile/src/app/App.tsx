import "react-native-gesture-handler";
import { StatusBar } from "expo-status-bar";
import * as Notifications from "expo-notifications";
import * as SplashScreen from "expo-splash-screen";
import { useCallback, useEffect, useRef, useState } from "react";

// Keep the native splash visible until we've loaded fonts. The
// expo-splash-screen plugin disables Expo's default auto-hide, so the
// app must explicitly hide the splash itself (see effect below).
SplashScreen.preventAutoHideAsync().catch(() => {
  /* no-op — preventAutoHideAsync rejects if the splash has already
     been hidden, e.g. during fast refresh. Safe to ignore. */
});
import { ActivityIndicator, AppState, StyleSheet, Text, View } from "react-native";
import { SafeAreaProvider, useSafeAreaInsets } from "react-native-safe-area-context";
import { onlineManager, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import NetInfo from "@react-native-community/netinfo";
import {
  NavigationContainer,
  NavigationState,
  PartialState,
  useNavigationContainerRef,
} from "@react-navigation/native";
import { useFonts } from "expo-font";
import {
  Manrope_400Regular,
  Manrope_500Medium,
  Manrope_600SemiBold,
  Manrope_700Bold,
} from "@expo-google-fonts/manrope";
import {
  SpaceGrotesk_500Medium,
  SpaceGrotesk_600SemiBold,
  SpaceGrotesk_700Bold,
} from "@expo-google-fonts/space-grotesk";
import { JetBrainsMono_400Regular } from "@expo-google-fonts/jetbrains-mono";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { AppNavigator, RootStackParamList } from "./AppNavigator";
import { appLinking, extractChallengeIdFromNotificationResponse } from "./linking";
import {
  consumePendingPushSyncSignal,
  initializeNotificationRuntime,
  PushSyncSignal,
  setForegroundNotificationHandler,
  setPushSyncHandler,
} from "../lib/notifications/pushNotifications";
import { ToastOverlay, type ToastState } from "../components/ToastOverlay";
import { signalApprovalStateMayHaveChanged } from "../lib/notifications/approvalRefreshSignal";
import { AuthSessionProvider } from "../features/auth/AuthSessionContext";
import { MobileConsentProvider } from "../lib/consent";
import { capture } from "../lib/telemetry";
import type { PushType } from "../lib/telemetry-schema";
import { GestureHandlerRootView } from "react-native-gesture-handler";
import type { BottomNavV2Tab } from "../components/BottomNavV2";
// import { NyxSheet } from "../features/nyx/NyxSheet"; // TODO: re-enable when chat is ready
import { ThemeProvider, useTheme } from "../theme/ThemeContext";

// Wire TanStack Query's online state to NetInfo so queries pause when offline
// and auto-refetch when connectivity returns.
onlineManager.setEventListener((setOnline) => {
  return NetInfo.addEventListener((state) => {
    setOnline(!!state.isConnected);
  });
});

const queryClient = new QueryClient();

function refreshQueryCacheFromPushSignal(signal: PushSyncSignal | null) {
  // Notification callbacks can run while the app is inactive. Mark cached
  // approval data stale here, but leave network work to the focused Activity
  // subscriber (foreground) or the existing active-query resume sweep.
  void queryClient.invalidateQueries({
    queryKey: ["challenges"],
    refetchType: "none",
  });
  void queryClient.invalidateQueries({
    queryKey: ["approvals"],
    refetchType: "none",
  });
  if (signal) {
    void queryClient.invalidateQueries({
      queryKey: ["challenge", signal.challengeId],
      // The detail query owns this key and historically refreshed live. Keep
      // that behavior in the foreground, while the focused signal subscriber
      // handles background/offline catch-up without starting network work.
      refetchType:
        AppState.currentState === "active" && onlineManager.isOnline()
          ? "active"
          : "none",
    });
  }
  signalApprovalStateMayHaveChanged();
}

function getActiveRouteName(
  state: NavigationState | PartialState<NavigationState> | undefined
): string | undefined {
  if (!state || state.routes.length === 0) return undefined;

  const index = state.index ?? 0;
  const route = state.routes[index];
  if (!route) return undefined;
  const nested = route.state as NavigationState | PartialState<NavigationState> | undefined;
  if (nested) {
    return getActiveRouteName(nested);
  }

  return route.name;
}

export default function App() {
  const navigationRef = useNavigationContainerRef<RootStackParamList>();
  const [currentRouteName, setCurrentRouteName] = useState<string | undefined>(undefined);
  const [isNyxOpen, setIsNyxOpen] = useState(false);
  const pendingChallengeFromTapRef = useRef<string | null>(null);
  const lastAppStateRef = useRef(AppState.currentState);
  const [pushToast, setPushToast] = useState<ToastState | null>(null);

  const flushPendingChallengeTapNavigation = useCallback(() => {
    if (!navigationRef.isReady()) return;

    const pendingChallengeId = pendingChallengeFromTapRef.current;
    if (!pendingChallengeId) return;

    const rootState = navigationRef.getRootState();
    if (!rootState?.routeNames?.includes("Activity")) {
      return;
    }

    pendingChallengeFromTapRef.current = null;
    navigationRef.navigate("Activity", { challengeId: pendingChallengeId });
  }, [navigationRef]);

  // Tapping the in-app toast resolves to exactly the same navigation as
  // tapping an OS notification, including the "Activity route not mounted
  // yet" deferral -- the pending id survives until the navigator is ready
  // and `onStateChange` flushes it.
  const openChallengeFromNotification = useCallback(
    (challengeId: string) => {
      pendingChallengeFromTapRef.current = challengeId;
      flushPendingChallengeTapNavigation();
    },
    [flushPendingChallengeTapNavigation]
  );

  const [fontsLoaded] = useFonts({
    Manrope_400Regular,
    Manrope_500Medium,
    Manrope_600SemiBold,
    Manrope_700Bold,
    SpaceGrotesk_500Medium,
    SpaceGrotesk_600SemiBold,
    SpaceGrotesk_700Bold,
    JetBrainsMono_400Regular,
  });
  const [fontLoadTimeout, setFontLoadTimeout] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => setFontLoadTimeout(true), 8000);
    return () => clearTimeout(t);
  }, []);

  // Hide the native splash once we have enough state to render the UI.
  // expo-splash-screen plugin disables the auto-hide, so without this
  // call the splash never goes away.
  const ready = fontsLoaded || fontLoadTimeout;
  useEffect(() => {
    if (ready) {
      SplashScreen.hideAsync().catch(() => {
        /* already hidden — fine */
      });
    }
  }, [ready]);

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void initializeNotificationRuntime()
      .then((unsubscribe) => {
        if (disposed) {
          unsubscribe();
          return;
        }
        cleanup = unsubscribe;
      })
      .catch((error) => {
        if (__DEV__) console.warn("[push] notification runtime bootstrap failed", error);
      });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  // Emit `mobile.push_received` — narrow categorization only. We never
  // read the notification body; only infer the `type` from the structured
  // data field (set by the backend push payload) and snapshot the
  // current AppState to classify foreground vs background delivery.
  useEffect(() => {
    const classifyPushType = (data: unknown): PushType => {
      if (!data || typeof data !== "object") return "other";
      const typeRaw = (data as { type?: unknown }).type;
      if (typeof typeRaw === "string" && typeRaw === "approval_request") {
        return "approval_request";
      }
      // Backend payloads may omit `type` on the approval path but still
      // ship a `challenge_id`/`request_id`, which uniquely identifies
      // an approval request. Anything else (decision ack, expiry)
      // is lumped into "other".
      if (!typeRaw) {
        const challengeId =
          (data as { challenge_id?: unknown }).challenge_id ??
          (data as { challengeId?: unknown }).challengeId ??
          (data as { request_id?: unknown }).request_id;
        if (typeof challengeId === "string" && challengeId.length > 0) {
          return "approval_request";
        }
      }
      return "other";
    };

    // NOTE: `addNotificationReceivedListener` only fires while the JS
    // runtime is active -- i.e., the app is foregrounded or briefly
    // active in the background. Pushes delivered while the app is
    // terminated, or shown by the OS without waking JS, are NOT observed
    // here. This listener therefore reliably measures ONLY
    // `app_state: "foreground"` receipts. Background/quit delivery
    // observability would require a native notification-service
    // extension on iOS and a foreground service on Android, which is
    // out of scope for this sweep. See docs/TELEMETRY.md §5.3. The
    // `app_state` prop is still populated from `AppState.currentState`
    // for honest emission when the listener does happen to catch a
    // backgrounded-but-live runtime; under normal conditions it reads
    // "foreground".
    const subscription = Notifications.addNotificationReceivedListener((notification) => {
      try {
        const pushType = classifyPushType(notification.request.content.data);
        const appState =
          AppState.currentState === "active" ? "foreground" : "background";
        capture({
          name: "mobile.push_received",
          props: { type: pushType, app_state: appState },
        });
      } catch {
        // never break push pipeline on telemetry failure
      }
    });

    return () => {
      subscription.remove();
    };
  }, []);

  useEffect(() => {
    const handleResponse = (response: Notifications.NotificationResponse | null) => {
      const challengeId = extractChallengeIdFromNotificationResponse(response);
      if (!challengeId) return;

      pendingChallengeFromTapRef.current = challengeId;
      flushPendingChallengeTapNavigation();
    };

    const responseSubscription = Notifications.addNotificationResponseReceivedListener((response) => {
      handleResponse(response);
    });

    void Notifications.getLastNotificationResponseAsync()
      .then((response) => {
        handleResponse(response);
      })
      .catch((error) => {
        if (__DEV__) {
          console.warn("[push] getLastNotificationResponseAsync failed", error);
        }
      });

    return () => {
      responseSubscription.remove();
    };
  }, [flushPendingChallengeTapNavigation]);

  // In-app replacement for the OS banner suppressed while the app is
  // foregrounded (see `configureNotificationHandler`). Without this, a push
  // arriving with the app open would be silent to the user. Reuses the same
  // toast the rest of the app uses; the backend's push body is boilerplate
  // ("A service is requesting access"), so the title alone loses nothing.
  useEffect(() => {
    return setForegroundNotificationHandler((notification) => {
      const message = notification.title ?? notification.body;
      if (!message) return;

      const { challengeId } = notification;
      setPushToast({
        message,
        kind: "info",
        action: challengeId
          ? {
              label: "Review",
              onPress: () => openChallengeFromNotification(challengeId),
            }
          : undefined,
      });
    });
  }, [openChallengeFromNotification]);

  useEffect(() => {
    const onPushSyncSignal = (signal: PushSyncSignal | null) => {
      refreshQueryCacheFromPushSignal(signal);
    };

    const disposePushSyncHandler = setPushSyncHandler(onPushSyncSignal);

    const consumePendingSignal = async () => {
      const pendingSignal = await consumePendingPushSyncSignal();
      if (pendingSignal) {
        refreshQueryCacheFromPushSignal(pendingSignal);
      }
    };

    void consumePendingSignal();

    const appStateSubscription = AppState.addEventListener("change", (nextState) => {
      const previousState = lastAppStateRef.current;
      lastAppStateRef.current = nextState;

      const resumedFromBackground =
        (previousState === "background" || previousState === "inactive") &&
        nextState === "active";

      if (!resumedFromBackground) return;

      void consumePendingSignal();
      void queryClient.refetchQueries({ type: "active" });
    });

    return () => {
      disposePushSyncHandler();
      appStateSubscription.remove();
    };
  }, []);

  const canShowApp = fontsLoaded || fontLoadTimeout;
  if (!canShowApp) {
    return (
      <View style={appLoadingStyles.container}>
        <StatusBar style="light" />
        <ActivityIndicator size="large" color="#9775fa" />
        <Text style={appLoadingStyles.text}>Loading...</Text>
      </View>
    );
  }

  return (
    <AppErrorBoundary>
      <GestureHandlerRootView style={appRootStyles.fill}>
        <SafeAreaProvider>
          <QueryClientProvider client={queryClient}>
            <ThemeProvider>
              <MobileConsentProvider>
                <AuthSessionProvider>
                  <ThemedAppShell
                    navigationRef={navigationRef}
                    currentRouteName={currentRouteName}
                    setCurrentRouteName={setCurrentRouteName}
                    flushPendingChallengeTapNavigation={flushPendingChallengeTapNavigation}
                    isNyxOpen={isNyxOpen}
                    setIsNyxOpen={setIsNyxOpen}
                    pushToast={pushToast}
                  />
                </AuthSessionProvider>
              </MobileConsentProvider>
            </ThemeProvider>
          </QueryClientProvider>
        </SafeAreaProvider>
      </GestureHandlerRootView>
    </AppErrorBoundary>
  );
}

function ThemedAppShell({
  navigationRef,
  currentRouteName,
  setCurrentRouteName,
  flushPendingChallengeTapNavigation,
  isNyxOpen,
  setIsNyxOpen,
  pushToast,
}: {
  navigationRef: ReturnType<typeof useNavigationContainerRef<RootStackParamList>>;
  currentRouteName: string | undefined;
  setCurrentRouteName: (name: string | undefined) => void;
  flushPendingChallengeTapNavigation: () => void;
  isNyxOpen: boolean;
  setIsNyxOpen: (open: boolean) => void;
  pushToast: ToastState | null;
}) {
  const { mode } = useTheme();
  const insets = useSafeAreaInsets();

  return (
    <>
      <NavigationContainer
        ref={navigationRef}
        linking={appLinking}
        onReady={() => {
          const routeName = getActiveRouteName(navigationRef.getRootState());
          setCurrentRouteName(routeName);
          flushPendingChallengeTapNavigation();
        }}
        onStateChange={(state) => {
          const routeName = getActiveRouteName(state);
          setCurrentRouteName(routeName);
          flushPendingChallengeTapNavigation();
        }}
      >
        <StatusBar style={mode === "dark" ? "light" : "dark"} />
        <AppNavigator
          currentRouteName={currentRouteName}
          onMainTabPress={(tab: BottomNavV2Tab) => {
            if (!navigationRef.isReady()) return;
            capture({
              name: "ui.mobile_nav_target_opened",
              props: { target: tab, source: "tab" },
            });
            if (tab === "activity") navigationRef.navigate("Activity");
            if (tab === "account") navigationRef.navigate("AccountSettings");
          }}
          // onNyxPress={() => setIsNyxOpen(true)} // TODO: re-enable when chat is ready
        />
      </NavigationContainer>
      {/* ToastOverlay anchors to `top: 0` of its parent and relies on the
          screen's SafeAreaView for inset. Mounted globally it has no such
          parent, so give it explicit bounds below the notch. */}
      <View
        style={[pushToastStyles.overlay, { top: insets.top }]}
        pointerEvents="box-none"
      >
        <ToastOverlay toast={pushToast} duration={6000} />
      </View>
      {/* <NyxSheet isOpen={isNyxOpen} onClose={() => setIsNyxOpen(false)} /> */}{/* TODO: re-enable when chat is ready */}
    </>
  );
}

const appLoadingStyles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#07060e",
    justifyContent: "center",
    alignItems: "center",
    gap: 16,
  },
  text: {
    color: "#e8e4f0",
    fontSize: 16,
  },
});

const pushToastStyles = StyleSheet.create({
  overlay: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
  },
});

const appRootStyles = StyleSheet.create({
  fill: {
    flex: 1,
  },
});
