import { Pressable, StyleSheet, useWindowDimensions, View } from "react-native";
import { Minus, Plus } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { useChatLayout } from "./ChatLayoutContext";
import {
  chatLayoutBounds,
  offersChatWidth,
  type ChatLayout,
} from "./chatLayout";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
import { ChatBubbleSettings } from "./ChatBubbleSettings";

/** Settings › Display groups for chat bubbles and chat layout. */
export function ChatDisplaySettings() {
  const { colors } = useTheme();
  const { width, fontScale } = useWindowDimensions();
  const stacked = width / fontScale < 340;
  const { layout, update, reset } = useChatLayout();
  function control(key: keyof ChatLayout, label: string, step: number) {
    const [min, max] = chatLayoutBounds[key];
    const stepper = (direction: -1 | 1) => {
      const disabled = direction < 0 ? layout[key] <= min : layout[key] >= max;
      const Glyph = direction < 0 ? Minus : Plus;
      return (
        <Pressable
          key={direction}
          accessibilityRole="button"
          accessibilityLabel={`${direction < 0 ? "Decrease" : "Increase"} ${label.toLowerCase()}`}
          accessibilityState={{ disabled }}
          disabled={disabled}
          onPress={() => update({ [key]: layout[key] + direction * step })}
          style={[
            styles.step,
            { backgroundColor: colors.navActive, opacity: disabled ? 0.4 : 1 },
          ]}
        >
          <Glyph size={22} color={colors.textPrimary} accessible={false} />
        </Pressable>
      );
    };
    return (
      <View style={[styles.row, stacked && styles.stacked]} key={key}>
        <Text
          style={[
            styles.label,
            stacked && styles.stackedLabel,
            { color: colors.textPrimary },
          ]}
        >
          {label}
        </Text>
        <View style={styles.controls}>
          {stepper(-1)}
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.value, { color: colors.textSecondary }]}
          >
            {layout[key]} pt
          </Text>
          {stepper(1)}
        </View>
      </View>
    );
  }
  return (
    <View style={styles.groups}>
      <ChatBubbleSettings />
      <SettingsGroup
        title="Chat layout"
        inset={false}
        footer="Saved on this device for every conversation. The composer grows with your draft up to this height."
      >
        {offersChatWidth(width) &&
          control("columnWidth", "Chat column width", 40)}
        {control("composerMaxHeight", "Maximum composer height", 20)}
        <SettingsRow centered label="Reset chat layout" onPress={reset} />
      </SettingsGroup>
    </View>
  );
}
const styles = StyleSheet.create({
  groups: { gap: spacing.huge },
  row: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: spacing.sm,
    minHeight: 52,
    paddingHorizontal: spacing.xxl,
    paddingVertical: spacing.xs,
  },
  stacked: {
    flexDirection: "column",
    alignItems: "stretch",
    paddingVertical: spacing.md,
  },
  label: { ...typeScale.label, flex: 1, minWidth: 0, flexShrink: 1 },
  stackedLabel: { flex: undefined, width: "100%" },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    maxWidth: "100%",
    gap: spacing.xs,
  },
  value: {
    ...typeScale.label,
    minWidth: 52,
    maxWidth: "100%",
    flexShrink: 1,
    textAlign: "center",
  },
  step: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
});
