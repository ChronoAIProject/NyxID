import { StatusBar } from "expo-status-bar";
import { useEffect, useState } from "react";
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
import { AppNavigator, RootStackParamList } from "./AppNavigator";
import { appLinking } from "./linking";
import { initializeNotificationRuntime } from "../lib/notifications/pushNotifications";
import { AuthSessionProvider } from "../features/auth/AuthSessionContext";
import { BottomNavTab } from "../components/BottomNav";

const queryClient = new QueryClient();

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

  const [fontsLoaded] = useFonts({
    Manrope_500Medium,
    Manrope_600SemiBold,
    Manrope_700Bold,
    SpaceGrotesk_600SemiBold,
    SpaceGrotesk_700Bold,
  });

  useEffect(() => {
    let disposed = false;
    let cleanup: (() => void) | undefined;

    void initializeNotificationRuntime().then((unsubscribe) => {
      if (disposed) {
        unsubscribe();
        return;
      }
      cleanup = unsubscribe;
    });

    return () => {
      disposed = true;
      cleanup?.();
    };
  }, []);

  if (!fontsLoaded) {
    return null;
  }

  return (
    <SafeAreaProvider>
      <QueryClientProvider client={queryClient}>
        <AuthSessionProvider>
          <NavigationContainer
            ref={navigationRef}
            linking={appLinking}
            onReady={() => {
              const routeName = getActiveRouteName(navigationRef.getRootState());
              setCurrentRouteName(routeName);
            }}
            onStateChange={(state) => {
              const routeName = getActiveRouteName(state);
              setCurrentRouteName(routeName);
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
  );
}
