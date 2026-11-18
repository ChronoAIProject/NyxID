import { ActivityIndicator, Pressable, StyleSheet } from "react-native";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, typeScale } from "../../theme/designTokens";

/**
 * The commit action of an edit page, top right of the bar as on iOS. Content
 * pages pass it through `useWorkspaceBack({ options: { headerRight } })` so the
 * workspace header and the action are applied together.
 */
export function SettingsHeaderButton({
  label = "Save",
  enabled,
  busy = false,
  onPress,
}: {
  label?: string;
  enabled: boolean;
  busy?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  if (busy) return <ActivityIndicator color={colors.primary} />;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !enabled }}
      disabled={!enabled}
      hitSlop={12}
      onPress={onPress}
      style={styles.action}
    >
      <Text
        style={[
          styles.label,
          { color: enabled ? colors.primary : colors.textMuted },
        ]}
      >
        {label}
      </Text>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  action: { minHeight: 44, justifyContent: "center", paddingHorizontal: 4 },
  label: {
    ...typeScale.label,
    fontFamily: fonts.bodySemi,
    fontWeight: "600",
  },
});
