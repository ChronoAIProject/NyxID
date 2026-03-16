import { StatusBar } from "expo-status-bar";
import * as Notifications from "expo-notifications";
import { useCallback, useEffect, useRef, useState } from "react";
import { ActivityIndicator, AppState, StyleSheet, Text, View } from "react-native";
import { SafeAreaProvider } from "react-native-safe-area-context";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
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
import { AuthSessionProvider } from "../features/auth/AuthSessionContext";
import { BottomNavTab } from "../components/BottomNav";

const queryClient = new QueryClient();

function refreshQueryCacheFromPushSignal(signal: PushSyncSignal) {
  void queryClient.invalidateQueries({ queryKey: ["challenges"] });
  void queryClient.invalidateQueries({ queryKey: ["approvals"] });
  void queryClient.invalidateQueries({ queryKey: ["challenge", signal.challengeId] });
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
  const pendingChallengeFromTapRef = useRef<string | null>(null);
  const lastAppStateRef = useRef(AppState.currentState);

  const flushPendingChallengeTapNavigation = useCallback(() => {
    if (!navigationRef.isReady()) return;

    const pendingChallengeId = pendingChallengeFromTapRef.current;
    if (!pendingChallengeId) return;

    const rootState = navigationRef.getRootState();
    if (!rootState?.routeNames?.includes("ChallengeDetail")) {
      return;
    }

    pendingChallengeFromTapRef.current = null;
    navigationRef.navigate("ChallengeDetail", { challengeId: pendingChallengeId });
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
      <View style={appRootStyles.fill}>
        <SafeAreaProvider>
          <QueryClientProvider client={queryClient}>
            <AuthSessionProvider>
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
                <StatusBar style="light" />
                <AppNavigator
                  currentRouteName={currentRouteName}
                  onMainTabPress={(tab: BottomNavTab) => {
                    if (!navigationRef.isReady()) return;
                    if (tab === "dashboard") navigationRef.navigate("Dashboard");
                    if (tab === "inbox") navigationRef.navigate("Inbox");
                    if (tab === "approvals") navigationRef.navigate("Approvals");
                    if (tab === "account") navigationRef.navigate("AccountSettings");
                  }}
                />
              </NavigationContainer>
            </AuthSessionProvider>
          </QueryClientProvider>
        </SafeAreaProvider>
      </View>
    </AppErrorBoundary>
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
    backgroundColor: "#10101A",
  },
});
