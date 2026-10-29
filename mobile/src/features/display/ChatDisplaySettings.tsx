import { Pressable, StyleSheet, useWindowDimensions, View } from "react-native";
import { Minus, Plus } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useChatLayout } from "./ChatLayoutContext";
import {
  chatLayoutBounds,
  offersChatWidth,
  type ChatLayout,
} from "./chatLayout";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { ChatBubbleSettings } from "./ChatBubbleSettings";

export function ChatDisplaySettings() {
  const { colors } = useTheme();
  const { width, fontScale } = useWindowDimensions();
  const stacked = width / fontScale < 340;
  const { layout, update, reset } = useChatLayout();
  function control(key: keyof ChatLayout, label: string, step: number) {
    const [min, max] = chatLayoutBounds[key];
    return (
      <View style={[styles.setting, stacked && styles.stacked]} key={key}>
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
          {([-1, 1] as const).map((direction) => {
            const disabled =
              direction < 0 ? layout[key] <= min : layout[key] >= max;
            return (
              <Pressable
                key={direction}
                accessibilityRole="button"
                accessibilityLabel={`${direction < 0 ? "Decrease" : "Increase"} ${label.toLowerCase()}`}
                accessibilityState={{ disabled }}
                disabled={disabled}
                onPress={() =>
                  update({ [key]: layout[key] + direction * step })
                }
                style={[
                  styles.step,
                  {
                    backgroundColor: colors.navActive,
                    opacity: disabled ? 0.4 : 1,
                  },
                ]}
              >
                {direction < 0 ? (
                  <Minus
                    size={22}
                    color={colors.textPrimary}
                    accessible={false}
                  />
                ) : (
                  <Plus
                    size={22}
                    color={colors.textPrimary}
                    accessible={false}
                  />
                )}
              </Pressable>
            );
          })}
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.value, { color: colors.textSecondary }]}
          >
            {layout[key]} pt
          </Text>
        </View>
      </View>
    );
  }
  return (
    <View
      style={[
        styles.card,
        { backgroundColor: colors.card, borderColor: colors.border },
      ]}
    >
      <Text
        accessibilityRole="header"
        style={[styles.title, { color: colors.textPrimary }]}
      >
        Display
      </Text>
      <Text style={{ color: colors.textSecondary }}>
        Chat layout is saved on this device and applies to every conversation.
      </Text>
      {offersChatWidth(width) &&
        control("columnWidth", "Chat column width", 40)}
      {control("composerMaxHeight", "Maximum composer height", 20)}
      <Text style={{ color: colors.textSecondary }}>
        The composer grows with your draft and fits the space above the
        keyboard.
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Reset chat layout"
        onPress={reset}
        style={[styles.reset, { borderColor: colors.textMuted }]}
      >
        <Text style={{ color: colors.textPrimary }}>Reset chat layout</Text>
      </Pressable>
      <ChatBubbleSettings />
    </View>
  );
}
const styles = StyleSheet.create({
  card: { borderWidth: 1, borderRadius: 16, padding: 20, gap: 16 },
  title: { fontFamily: fonts.bodySemi, fontSize: 16 },
  setting: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
  },
  stacked: { flexDirection: "column", alignItems: "stretch" },
  label: { flex: 1, minWidth: 0, flexShrink: 1 },
  stackedLabel: { flex: undefined, width: "100%" },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    maxWidth: "100%",
    gap: 8,
  },
  value: { minWidth: 48, maxWidth: "100%", flexShrink: 1 },
  step: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
    flexShrink: 0,
  },
  reset: {
    minHeight: 44,
    borderWidth: 1,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
    padding: 8,
  },
});
