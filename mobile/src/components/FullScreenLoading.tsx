import { ActivityIndicator, StyleSheet, Text, View } from "react-native";
import { mobileTheme } from "../theme/mobileTheme";
import { radius, spacing, typeScale } from "../theme/designTokens";
import { MobileStatusBar } from "./MobileStatusBar";
import { ScreenContainer } from "./ScreenContainer";

type FullScreenLoadingProps = {
  title?: string;
  subtitle?: string;
};

export function FullScreenLoading({
  title = "Loading...",
  subtitle = "Syncing the latest data, please wait.",
}: FullScreenLoadingProps) {
  return (
    <ScreenContainer>
      <MobileStatusBar />
      <View style={styles.center}>
        <View style={styles.card}>
          <ActivityIndicator size="small" color={mobileTheme.primary} />
          <Text style={styles.title}>{title}</Text>
          <Text style={styles.subtitle}>{subtitle}</Text>
        </View>
      </View>
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  center: {
    flex: 1,
    justifyContent: "center",
    alignItems: "center",
    paddingBottom: 72,
  },
  card: {
    width: "100%",
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: mobileTheme.borderSoft,
    backgroundColor: mobileTheme.card,
    paddingVertical: spacing.xxl,
    paddingHorizontal: spacing.xl,
    alignItems: "center",
    gap: spacing.sm,
  },
  title: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
  },
  subtitle: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
    textAlign: "center",
  },
});
