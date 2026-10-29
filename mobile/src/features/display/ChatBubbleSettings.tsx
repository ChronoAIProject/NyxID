import { Pressable, StyleSheet, View } from "react-native";
import { Check } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useTheme } from "../../theme/ThemeContext";
import { lightBubblePalettes, userBubbleCorners, type LightBubblePalette } from "../../theme/chatAppearance";
import { fonts } from "../../theme/designTokens";
import { lightColors } from "../../theme/mobileTheme";

export function ChatBubbleSettings() {
  const { colors, lightBubblePalette, setLightBubblePalette } = useTheme();
  const selected = lightBubblePalettes[lightBubblePalette];
  return (
    <View style={styles.section}>
      <Text style={[styles.title, { color: colors.textPrimary }]}>Light chat bubbles</Text>
      <Text style={{ color: colors.textSecondary }}>
        Choose a background for your messages in light mode. Saved on this device.
      </Text>
      <View accessibilityRole="radiogroup" accessibilityLabel="Light chat bubble color" style={styles.options}>
        {(Object.keys(lightBubblePalettes) as LightBubblePalette[]).map((key) => {
          const palette = lightBubblePalettes[key];
          return (
            <Pressable
              key={key}
              accessibilityRole="radio"
              accessibilityLabel={`${palette.label} chat bubbles`}
              accessibilityState={{ checked: lightBubblePalette === key }}
              onPress={() => setLightBubblePalette(key)}
              style={[
                styles.option,
                {
                  backgroundColor: palette.background,
                  borderColor: lightBubblePalette === key ? colors.primary : colors.controlBorder,
                  borderWidth: lightBubblePalette === key ? 2 : 1,
                },
              ]}
            >
              <Text style={{ color: palette.foreground }}>{palette.label}</Text>
              {lightBubblePalette === key && <Check size={16} color={palette.foreground} accessible={false} />}
            </Pressable>
          );
        })}
      </View>
      <View style={styles.preview}>
        <Text style={styles.previewLabel}>Chat preview · light</Text>
        <View style={[styles.bubble, { backgroundColor: selected.background }]}>
          <Text style={{ color: selected.foreground }}>Show me the services I have connected.</Text>
        </View>
        <Text style={styles.previewText}>You can manage your connected services in Plugins.</Text>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  section: { gap: 12 },
  title: { fontFamily: fonts.bodySemi, fontSize: 14 },
  options: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  option: { minHeight: 44, paddingVertical: 10, paddingHorizontal: 16, borderRadius: 12, flexDirection: "row", alignItems: "center", gap: 8 },
  preview: { backgroundColor: lightColors.bg, borderColor: lightColors.border, borderWidth: 1, borderRadius: 12, padding: 12, gap: 16 },
  previewLabel: { color: lightColors.textPrimary, fontFamily: fonts.bodySemi, fontSize: 12 },
  previewText: { color: lightColors.textPrimary, fontSize: 14 },
  bubble: { ...userBubbleCorners, padding: 12, marginLeft: 24, alignSelf: "flex-end" },
});
