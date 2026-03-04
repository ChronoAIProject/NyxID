import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { mobileTheme } from "../../theme/mobileTheme";
import { flowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "RevokeSuccess">;

export function RevokeSuccessScreen({ navigation }: Props) {
  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="SUCCESS" tone="success" />
        <Text style={flowStyles.title}>Approved Sessions</Text>
        <Text style={flowStyles.subtitle}>
          Revoke completed. Future calls to this service will trigger challenge again.
        </Text>

        <View style={styles.emptyState}>
          <Text style={styles.emptyIcon}>✓</Text>
          <Text style={styles.emptyTitle}>Revoke Completed</Text>
          <Text style={styles.emptySub}>No active approval remains for this request pattern.</Text>
        </View>

        <View style={styles.toastCard}>
          <Text style={styles.toastTitle}>Security update applied</Text>
          <Text style={styles.toastSub}>New requests now require explicit challenge approval.</Text>
        </View>

        <View style={flowStyles.actionWrap}>
          <PrimaryButton label="Back to Approvals" onPress={() => navigation.replace("Approvals")} />
          <PrimaryButton label="Go Dashboard" kind="ghost" onPress={() => navigation.replace("Dashboard")} />
        </View>
      </ScrollView>
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  emptyState: {
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: mobileTheme.borderSoft,
    backgroundColor: mobileTheme.card,
    paddingVertical: spacing.huge,
    paddingHorizontal: spacing.xxl,
    alignItems: "center",
    gap: spacing.sm,
  },
  emptyIcon: {
    color: mobileTheme.success,
    fontSize: 24,
    fontWeight: "700",
  },
  emptyTitle: {
    color: mobileTheme.textPrimary,
    fontSize: 17,
    fontFamily: typeScale.h2.fontFamily,
    fontWeight: "700",
  },
  emptySub: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
    textAlign: "center",
    fontSize: 13,
    lineHeight: 18,
  },
  toastCard: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: mobileTheme.borderSoft,
    backgroundColor: "#0D0B14",
    paddingVertical: spacing.lg,
    paddingHorizontal: spacing.xl,
    gap: spacing.xs,
  },
  toastTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.caption,
    fontWeight: "700",
    fontSize: 13,
  },
  toastSub: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
  },
});
