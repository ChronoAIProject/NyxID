import { createNativeStackNavigator } from "@react-navigation/native-stack";
import { StyleSheet, View } from "react-native";
import { AuthHomeScreen } from "../features/auth/AuthHomeScreen";
import { useAuthSession } from "../features/auth/AuthSessionContext";
import { AccountSettingsScreen } from "../features/account/AccountSettingsScreen";
import { DashboardScreen } from "../features/challenges/DashboardScreen";
import { ChallengesInboxScreen } from "../features/challenges/ChallengesInboxScreen";
import { ChallengeDetailScreen } from "../features/challenges/ChallengeDetailScreen";
import { ChallengeMinimalScreen } from "../features/challenges/ChallengeMinimalScreen";
import { ChallengeOptionsScreen } from "../features/challenges/ChallengeOptionsScreen";
import { RevokeConfirmScreen } from "../features/challenges/RevokeConfirmScreen";
import { RevokeSuccessScreen } from "../features/challenges/RevokeSuccessScreen";
import { ApprovalsScreen } from "../features/approvals/ApprovalsScreen";
import { TermsOfServiceScreen } from "../features/legal/TermsOfServiceScreen";
import { PrivacyPolicyScreen } from "../features/legal/PrivacyPolicyScreen";
import { FullScreenLoading } from "../components/FullScreenLoading";
import { BottomNav, BottomNavTab } from "../components/BottomNav";
import { mobileTheme } from "../theme/mobileTheme";
import { spacing } from "../theme/designTokens";

export type RootStackParamList = {
  Auth: undefined;
  Dashboard: undefined;
  AccountSettings: undefined;
  Inbox: undefined;
  ChallengeMinimal: { challengeId: string };
  ChallengeOptions: { challengeId: string };
  ChallengeDetail: { challengeId: string };
  Approvals: undefined;
  RevokeConfirm: { approvalId: string };
  RevokeSuccess: undefined;
  TermsOfService: undefined;
  PrivacyPolicy: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();

type AppNavigatorProps = {
  currentRouteName?: string;
  onMainTabPress?: (tab: BottomNavTab) => void;
};

function resolveActiveMainTab(routeName?: string): BottomNavTab {
  if (!routeName) return "dashboard";
  if (routeName === "Dashboard") return "dashboard";
  if (routeName === "AccountSettings") return "account";
  if (routeName === "Inbox") return "inbox";
  if (routeName === "Approvals") return "approvals";
  if (routeName === "RevokeConfirm") return "approvals";
  if (routeName === "RevokeSuccess") return "approvals";
  if (routeName === "ChallengeMinimal") return "inbox";
  if (routeName === "ChallengeOptions") return "inbox";
  if (routeName === "ChallengeDetail") return "inbox";
  return "dashboard";
}

export function AppNavigator({ currentRouteName, onMainTabPress }: AppNavigatorProps) {
  const { isAuthenticated, isRestoring } = useAuthSession();
  const activeMainTab = resolveActiveMainTab(currentRouteName);
  const isLegalRoute = currentRouteName === "TermsOfService" || currentRouteName === "PrivacyPolicy";
  const showGlobalBottomNav = isAuthenticated && Boolean(onMainTabPress) && !isLegalRoute;

  if (isRestoring) {
    return <FullScreenLoading title="Restoring session..." subtitle="Validating local secure session" />;
  }

  return (
    <View style={styles.container}>
      <View style={styles.stackWrap}>
        <Stack.Navigator
          initialRouteName={isAuthenticated ? "Dashboard" : "Auth"}
          screenOptions={{
            headerShown: false,
            headerStyle: { backgroundColor: "#10101A" },
            headerTintColor: "#F0EEFF",
            contentStyle: { backgroundColor: "#10101A" },
          }}
        >
          {isAuthenticated ? (
            <>
              <Stack.Screen
                name="Dashboard"
                component={DashboardScreen}
                options={{ title: "Dashboard", animation: "none" }}
              />
              <Stack.Screen
                name="AccountSettings"
                component={AccountSettingsScreen}
                options={{ title: "Account Settings" }}
              />
              <Stack.Screen
                name="Inbox"
                component={ChallengesInboxScreen}
                options={{ title: "Challenges", animation: "none" }}
              />
              <Stack.Screen
                name="ChallengeMinimal"
                component={ChallengeMinimalScreen}
                options={{ title: "Challenge Minimal", animation: "none" }}
              />
              <Stack.Screen
                name="ChallengeOptions"
                component={ChallengeOptionsScreen}
                options={{ title: "Challenge Options", animation: "none" }}
              />
              <Stack.Screen
                name="ChallengeDetail"
                component={ChallengeDetailScreen}
                options={{ title: "Challenge Detail", animation: "none" }}
              />
              <Stack.Screen
                name="Approvals"
                component={ApprovalsScreen}
                options={{ title: "Approvals", animation: "none" }}
              />
              <Stack.Screen
                name="RevokeConfirm"
                component={RevokeConfirmScreen}
                options={{ title: "Revoke Confirm" }}
              />
              <Stack.Screen
                name="RevokeSuccess"
                component={RevokeSuccessScreen}
                options={{ title: "Revoke Success" }}
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
            options={{ title: "Terms of Service", animation: "none" }}
          />
          <Stack.Screen
            name="PrivacyPolicy"
            component={PrivacyPolicyScreen}
            options={{ title: "Privacy Policy", animation: "none" }}
          />
        </Stack.Navigator>
      </View>
      {showGlobalBottomNav ? (
        <View style={styles.bottomWrap}>
          <BottomNav active={activeMainTab} onTabPress={(tab) => onMainTabPress?.(tab)} />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: mobileTheme.bg,
  },
  stackWrap: {
    flex: 1,
  },
  bottomWrap: {
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.xxxl,
  },
});
