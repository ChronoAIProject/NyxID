import { Pressable, StyleSheet, View } from "react-native";
import { Check } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { SettingsGroup } from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import { bubblePalettes, userBubbleCorners, type BubblePalette } from "../../theme/chatAppearance";
import { spacing } from "../../theme/designTokens";

/** Bubble colour picker with a single preview in the current theme. */
export function ChatBubbleSettings() {
  const { colors, mode, bubblePalette, setBubblePalette } = useTheme();
  const tone = (key: BubblePalette) =>
    mode === "dark" ? bubblePalettes[key].dark : bubblePalettes[key].light;
  const selected = tone(bubblePalette);
  return (
    <SettingsGroup
      title="Chat bubbles"
      inset={false}
      footer="The color of your messages. Saved on this device."
    >
      <View style={styles.content}>
        <View accessibilityRole="radiogroup" accessibilityLabel="Chat bubble color" style={styles.options}>
          {(Object.keys(bubblePalettes) as BubblePalette[]).map((key) => {
            const swatch = tone(key);
            const checked = bubblePalette === key;
            return (
              <Pressable
                key={key}
                accessibilityRole="radio"
                accessibilityLabel={`${bubblePalettes[key].label} chat bubbles`}
                accessibilityState={{ checked }}
                onPress={() => setBubblePalette(key)}
                style={[
                  styles.option,
                  {
                    backgroundColor: swatch.background,
                    borderColor: checked ? colors.primary : colors.controlBorder,
                    borderWidth: checked ? 2 : 1,
                  },
                ]}
              >
                <Text style={{ color: swatch.foreground }}>{bubblePalettes[key].label}</Text>
                {checked && <Check size={16} color={swatch.foreground} accessible={false} />}
              </Pressable>
            );
          })}
        </View>
        <View
          accessibilityLabel="Chat bubble preview"
          style={[styles.bubble, { backgroundColor: selected.background }]}
        >
          <Text style={{ color: selected.foreground }}>Show me my connected services.</Text>
        </View>
      </View>
    </SettingsGroup>
  );
}

const styles = StyleSheet.create({
  content: { padding: spacing.xxl, gap: spacing.xxl },
  options: { flexDirection: "row", flexWrap: "wrap", gap: spacing.sm },
  option: { minHeight: 44, paddingVertical: 10, paddingHorizontal: 14, borderRadius: 12, flexDirection: "row", alignItems: "center", gap: 6 },
  bubble: { ...userBubbleCorners, paddingVertical: 10, paddingHorizontal: 12, marginLeft: 48, alignSelf: "flex-end" },
});
