import { ActivityIndicator, StyleSheet, Text, View } from "react-native";
import { radius, spacing } from "../theme/designTokens";
import { MobileStatusBar } from "./MobileStatusBar";
import { ScreenContainer } from "./ScreenContainer";

// Use system font only so this screen always renders (e.g. when custom fonts not yet loaded).
const FALLBACK_PRIMARY = "#8B5CF6";
const FALLBACK_BG = "#10101A";
const FALLBACK_CARD = "#1A1A24";
const FALLBACK_BORDER = "#263042";
const FALLBACK_TEXT = "#F0EEFF";
const FALLBACK_MUTED = "#6A6480";

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
          <ActivityIndicator size="small" color={FALLBACK_PRIMARY} />
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
    borderColor: FALLBACK_BORDER,
    backgroundColor: FALLBACK_CARD,
    paddingVertical: spacing.xxl,
    paddingHorizontal: spacing.xl,
    alignItems: "center",
    gap: spacing.sm,
  },
  title: {
    color: FALLBACK_TEXT,
    fontSize: 16,
    fontWeight: "600",
  },
  subtitle: {
    color: FALLBACK_MUTED,
    fontSize: 13,
    textAlign: "center",
  },
});
