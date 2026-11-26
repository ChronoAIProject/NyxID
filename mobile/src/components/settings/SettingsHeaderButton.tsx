import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { ActivityIndicator, StyleSheet } from "react-native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { FloatingButton } from "../FloatingControl";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, typeScale } from "../../theme/designTokens";

/**
 * The commit action of an edit page, top right of the bar as on iOS, in the
 * bar's floating glass (the same FloatingButton as Menu/Back and the chat
 * bar's controls): a 44pt pill hugging its label. While saving, a spinner
 * takes the label's place in the same capsule, so the bar never jumps.
 * Content pages pass it through `useWorkspaceBack({ options: { headerRight } })`
 * and sheet pages through `useSettingsHeaderAction`, so the header and the
 * action are applied together.
 */
export function SettingsHeaderButton({
  label = translateApp("Save"),
  enabled,
  busy = false,
  onPress,
}: {
  label?: string;
  enabled: boolean;
  busy?: boolean;
  onPress: () => void;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <FloatingButton
      accessibilityLabel={label}
      // Busy keeps its spinner undimmed but takes no second press.
      accessibilityState={{ disabled: !enabled || busy, busy }}
      disabled={!enabled && !busy}
      onPress={busy ? undefined : onPress}
      style={styles.action}
    >
      {busy ? (
        <ActivityIndicator color={colors.primary} />
      ) : (
        <Text style={[styles.label, { color: colors.primary }]}>{label}</Text>
      )}
    </FloatingButton>
  );
}

const styles = StyleSheet.create({
  // The pill's own padding; the capsule and its 44pt minimum come from the
  // primitive.
  action: { paddingHorizontal: 14 },
  label: {
    ...typeScale.label,
    fontFamily: fonts.bodySemi,
    fontWeight: "600",
  },
});
