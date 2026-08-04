import { createNativeStackNavigator } from "@react-navigation/native-stack";
import { StyleSheet, View } from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { AuthHomeScreen } from "../features/auth/AuthHomeScreen";
import { useAuthSession } from "../features/auth/AuthSessionContext";
import { AccountSettingsScreen } from "../features/account/AccountSettingsScreen";
import { ActivityScreen } from "../features/activity/ActivityScreen";
import { ActivityDetailScreen } from "../features/activity/ActivityDetailScreen";
import { TermsOfServiceScreen } from "../features/legal/TermsOfServiceScreen";
import { PrivacyPolicyScreen } from "../features/legal/PrivacyPolicyScreen";
import { FullScreenLoading } from "../components/FullScreenLoading";
import { BottomNavV2, type BottomNavV2Tab } from "../components/BottomNavV2";
// import { NyxFAB } from "../components/NyxFAB"; // TODO: re-enable when chat is ready
import { useTheme } from "../theme/ThemeContext";
import { spacing } from "../theme/designTokens";

export type RootStackParamList = {
  Auth: undefined;
  Activity: { challengeId?: string } | undefined;
  ActivityDetail: { challengeId: string };
  AccountSettings: undefined;
  TermsOfService: undefined;
  PrivacyPolicy: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();

type AppNavigatorProps = {
  currentRouteName?: string;
  onMainTabPress?: (tab: BottomNavV2Tab) => void;
  onNyxPress?: () => void;
};

function resolveActiveMainTab(routeName?: string): BottomNavV2Tab {
  if (!routeName) return "activity";
  if (routeName === "Activity") return "activity";
  if (routeName === "ActivityDetail") return "activity";
  if (routeName === "AccountSettings") return "account";
  if (routeName === "TermsOfService") return "account";
  if (routeName === "PrivacyPolicy") return "account";
  return "activity";
}

export function AppNavigator({ currentRouteName, onMainTabPress, onNyxPress }: AppNavigatorProps) {
  const { isAuthenticated, isRestoring } = useAuthSession();
  const { colors } = useTheme();
  const activeMainTab = resolveActiveMainTab(currentRouteName);
  const isLegalRoute = currentRouteName === "TermsOfService" || currentRouteName === "PrivacyPolicy";
  const isDetailRoute = currentRouteName === "ActivityDetail";
  const showGlobalBottomNav = isAuthenticated && Boolean(onMainTabPress) && !isLegalRoute && !isDetailRoute;

  if (isRestoring) {
    return <FullScreenLoading title="Restoring session..." subtitle="Validating local secure session" />;
  }

  return (
    <View style={[styles.container, { backgroundColor: colors.bg }]}>
      <View style={styles.stackWrap}>
        <Stack.Navigator
          initialRouteName={isAuthenticated ? "Activity" : "Auth"}
          screenOptions={{
            headerShown: false,
            headerStyle: { backgroundColor: colors.bg },
            headerTintColor: colors.textPrimary,
            contentStyle: { backgroundColor: colors.bg },
          }}
        >
          {isAuthenticated ? (
            <>
              <Stack.Screen
                name="Activity"
                component={ActivityScreen}
                options={{ title: "Activity", animation: "slide_from_left" }}
              />
              <Stack.Screen
                name="ActivityDetail"
                component={ActivityDetailScreen}
                options={{ title: "Approval Detail", animation: "none" }}
              />
              <Stack.Screen
                name="AccountSettings"
                component={AccountSettingsScreen}
                options={{ title: "Account Settings", animation: "slide_from_right" }}
              />
            </>
          ) : (
            <>
              <Stack.Screen name="Auth" component={AuthHomeScreen} options={{ title: "NyxID Sign In" }} />
            </>
          )}
          <Stack.Screen
            name="TermsOfService"
            component={TermsOfServiceScreen}
            options={{ title: "Terms of Service", animation: "slide_from_left" }}
          />
          <Stack.Screen
            name="PrivacyPolicy"
            component={PrivacyPolicyScreen}
            options={{ title: "Privacy Policy", animation: "slide_from_left" }}
          />
        </Stack.Navigator>
      </View>
      {showGlobalBottomNav ? (
        <View style={styles.bottomOverlay} pointerEvents="box-none">
          <LinearGradient
            colors={[colors.bg + "00", colors.bg]}
            locations={[0, 1]}
            style={styles.fadeGradient}
            pointerEvents="none"
          />
          <View style={[styles.bottomWrap, { backgroundColor: colors.bg }]}>
            <View style={styles.navContainer}>
              <BottomNavV2 active={activeMainTab} onTabPress={(tab) => onMainTabPress?.(tab)} />
              {/* TODO: re-enable when chat is ready
              <View style={styles.fabPosition}>
                <NyxFAB onPress={onNyxPress} />
              </View>
              */}
            </View>
          </View>
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
  },
  stackWrap: {
    flex: 1,
  },
  bottomOverlay: {
    position: "absolute",
    left: 0,
    right: 0,
    bottom: 0,
  },
  fadeGradient: {
    height: 40,
  },
  bottomWrap: {
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.xxxl,
  },
  navContainer: {
    position: "relative",
  },
  fabPosition: {
    position: "absolute",
    top: -41.5,
    left: "50%",
    marginLeft: -28.5,
    overflow: "visible",
    height: 77,
    justifyContent: "flex-end",
  } as const,
});
