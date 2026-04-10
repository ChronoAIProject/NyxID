import "react-native-gesture-handler";
import { StatusBar } from "expo-status-bar";
import * as Notifications from "expo-notifications";
import { useCallback, useEffect, useRef, useState } from "react";
import { ActivityIndicator, AppState, StyleSheet, Text, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
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
  Manrope_500Medium,
  Manrope_600SemiBold,
  Manrope_700Bold,
} from "@expo-google-fonts/manrope";
import {
  SpaceGrotesk_600SemiBold,
  SpaceGrotesk_700Bold,
} from "@expo-google-fonts/space-grotesk";
import { AppErrorBoundary } from "./AppErrorBoundary";
import { AppNavigator, RootStackParamList } from "./AppNavigator";
import { appLinking, extractChallengeIdFromNotificationResponse } from "./linking";
import {
  consumePendingPushSyncSignal,
  initializeNotificationRuntime,
  PushSyncSignal,
  setPushSyncHandler,
} from "../lib/notifications/pushNotifications";
import { startPushPolling } from "../lib/notifications/pushPollingSignal";
import { AuthSessionProvider } from "../features/auth/AuthSessionContext";
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

function refreshQueryCacheFromPushSignal(signal: PushSyncSignal) {
  void queryClient.invalidateQueries({ queryKey: ["challenges"] });
  void queryClient.invalidateQueries({ queryKey: ["approvals"] });
  void queryClient.invalidateQueries({ queryKey: ["challenge", signal.challengeId] });
  startPushPolling();
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

  const [fontsLoaded] = useFonts({
    Manrope_500Medium,
    Manrope_600SemiBold,
    Manrope_700Bold,
    SpaceGrotesk_600SemiBold,
    SpaceGrotesk_700Bold,
  });
  const [fontLoadTimeout, setFontLoadTimeout] = useState(false);

  useEffect(() => {
    const t = setTimeout(() => setFontLoadTimeout(true), 8000);
    return () => clearTimeout(t);
  }, []);

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

  useEffect(() => {
    const onPushSyncSignal = (signal: PushSyncSignal) => {
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
        <ActivityIndicator size="large" color="#8B5CF6" />
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
              <AuthSessionProvider>
                <ThemedAppShell
                  navigationRef={navigationRef}
                  currentRouteName={currentRouteName}
                  setCurrentRouteName={setCurrentRouteName}
                  flushPendingChallengeTapNavigation={flushPendingChallengeTapNavigation}
                  isNyxOpen={isNyxOpen}
                  setIsNyxOpen={setIsNyxOpen}
                />
              </AuthSessionProvider>
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
}: {
  navigationRef: ReturnType<typeof useNavigationContainerRef<RootStackParamList>>;
  currentRouteName: string | undefined;
  setCurrentRouteName: (name: string | undefined) => void;
  flushPendingChallengeTapNavigation: () => void;
  isNyxOpen: boolean;
  setIsNyxOpen: (open: boolean) => void;
}) {
  const { mode } = useTheme();

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
            if (tab === "activity") navigationRef.navigate("Activity");
            if (tab === "account") navigationRef.navigate("AccountSettings");
          }}
          // onNyxPress={() => setIsNyxOpen(true)} // TODO: re-enable when chat is ready
        />
      </NavigationContainer>
      {/* <NyxSheet isOpen={isNyxOpen} onClose={() => setIsNyxOpen(false)} /> */}{/* TODO: re-enable when chat is ready */}
    </>
  );
}

const appLoadingStyles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: "#10101A",
    justifyContent: "center",
    alignItems: "center",
    gap: 16,
  },
  text: {
    color: "#F0EEFF",
    fontSize: 16,
  },
});

const appRootStyles = StyleSheet.create({
  fill: {
    flex: 1,
  },
});
