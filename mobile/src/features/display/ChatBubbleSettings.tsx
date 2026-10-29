import { Pressable, StyleSheet, View } from "react-native";
import { Check } from "lucide-react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useTheme } from "../../theme/ThemeContext";
import { bubblePalettes, userBubbleCorners, type BubblePalette } from "../../theme/chatAppearance";
import { fonts } from "../../theme/designTokens";
import { darkColors, lightColors, type ThemeColors } from "../../theme/mobileTheme";

function Preview({ label, page, bubble }: {
  label: string;
  page: ThemeColors;
  bubble: { background: string; foreground: string };
}) {
  return (
    <View style={[styles.preview, { backgroundColor: page.bg, borderColor: page.border }]}>
      <Text style={[styles.previewLabel, { color: page.textPrimary }]}>{label}</Text>
      <View style={[styles.bubble, { backgroundColor: bubble.background }]}>
        <Text style={{ color: bubble.foreground }}>Show me the services I have connected.</Text>
      </View>
      <Text style={[styles.previewText, { color: page.textPrimary }]}>
        You can manage your connected services in Plugins.
      </Text>
    </View>
  );
}

export function ChatBubbleSettings() {
  const { colors, mode, bubblePalette, setBubblePalette } = useTheme();
  const selected = bubblePalettes[bubblePalette];
  return (
    <View style={styles.section}>
      <Text style={[styles.title, { color: colors.textPrimary }]}>Chat bubbles</Text>
      <Text style={{ color: colors.textSecondary }}>
        Choose a color for your messages. Each choice has a light and a dark version. Saved on this device.
      </Text>
      <View accessibilityRole="radiogroup" accessibilityLabel="Chat bubble color" style={styles.options}>
        {(Object.keys(bubblePalettes) as BubblePalette[]).map((key) => {
          const palette = bubblePalettes[key];
          const swatch = mode === "dark" ? palette.dark : palette.light;
          const checked = bubblePalette === key;
          return (
            <Pressable
              key={key}
              accessibilityRole="radio"
              accessibilityLabel={`${palette.label} chat bubbles`}
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
              <Text style={{ color: swatch.foreground }}>{palette.label}</Text>
              {checked && <Check size={16} color={swatch.foreground} accessible={false} />}
            </Pressable>
          );
        })}
      </View>
      <Preview label="Chat preview · light" page={lightColors} bubble={selected.light} />
      <Preview label="Chat preview · dark" page={darkColors} bubble={selected.dark} />
    </View>
  );
}

const styles = StyleSheet.create({
  section: { gap: 12 },
  title: { fontFamily: fonts.bodySemi, fontSize: 14 },
  options: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  option: { minHeight: 44, paddingVertical: 10, paddingHorizontal: 16, borderRadius: 12, flexDirection: "row", alignItems: "center", gap: 8 },
  preview: { borderWidth: 1, borderRadius: 12, padding: 12, gap: 16 },
  previewLabel: { fontFamily: fonts.bodySemi, fontSize: 12 },
  previewText: { fontSize: 14 },
  bubble: { ...userBubbleCorners, padding: 12, marginLeft: 24, alignSelf: "flex-end" },
});
