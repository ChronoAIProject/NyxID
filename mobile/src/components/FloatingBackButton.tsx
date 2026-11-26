import { ChevronLeft } from "lucide-react-native";
import { StyleSheet } from "react-native";
import { translateApp } from "../i18n/appTranslation";
import { useAppLanguage } from "../i18n/LanguageProvider";
import { spacing } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import { FloatingButton } from "./FloatingControl";

/** A floating Back over a scrolling document: the shared glass button. */
export function FloatingBackButton({ onPress }: { onPress: () => void }) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <FloatingButton
      accessibilityLabel={translateApp("Back")}
      onPress={onPress}
      hitSlop={8}
      capsuleStyle={styles.capsule}
    >
      <ChevronLeft size={24} color={colors.textPrimary} />
    </FloatingButton>
  );
}

const styles = StyleSheet.create({
  capsule: { alignSelf: "flex-start", marginBottom: spacing.lg },
});
