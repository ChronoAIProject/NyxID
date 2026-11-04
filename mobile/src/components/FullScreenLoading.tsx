import { StyleSheet, Text, View } from "react-native";
import { fonts } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import { BrandNebula, BreathingMark } from "./brand/BrandNebula";
import { PortalMarkLogo } from "./brand/PortalMark";
import { ScreenContainer } from "./ScreenContainer";

type FullScreenLoadingProps = {
  title?: string;
  subtitle?: string;
};

/**
 * The brand splash while the app gets ready (restoring the saved session):
 * the portal mark breathing on drifting violet light and one quiet line,
 * no card or spinner chrome.
 */
export function FullScreenLoading({
  title = "Loading…",
  subtitle,
}: FullScreenLoadingProps) {
  const { colors } = useTheme();
  return (
    <ScreenContainer>
      <View
        accessibilityRole="progressbar"
        accessibilityLabel={subtitle ? `${title} ${subtitle}` : title}
        style={styles.center}
      >
        <BrandNebula height={520} />
        <BreathingMark>
          <PortalMarkLogo size={88} />
        </BreathingMark>
        <View style={styles.copy}>
          <Text style={[styles.title, { color: colors.textPrimary }]}>{title}</Text>
          {subtitle ? (
            <Text style={[styles.subtitle, { color: colors.textSecondary }]}>
              {subtitle}
            </Text>
          ) : null}
        </View>
      </View>
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  center: {
    flex: 1,
    alignItems: "center",
    justifyContent: "center",
    gap: 28,
    paddingBottom: 72,
  },
  copy: { alignItems: "center", gap: 6, paddingHorizontal: 32 },
  title: { fontFamily: fonts.displaySemi, fontSize: 20, lineHeight: 26, textAlign: "center" },
  subtitle: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18, textAlign: "center" },
});
