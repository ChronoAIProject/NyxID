import { StyleSheet, View } from "react-native";
import {
  SettingsCheckRow,
  SettingsGroup,
} from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import { bubblePalettes, type BubblePalette } from "../../theme/chatAppearance";

/** Bubble colour as an iOS choice list; the page preview shows the result. */
export function ChatBubbleSettings() {
  const { colors, mode, bubblePalette, setBubblePalette } = useTheme();
  const tone = (key: BubblePalette) =>
    mode === "dark" ? bubblePalettes[key].dark : bubblePalettes[key].light;
  return (
    <SettingsGroup
      title="Message bubble"
      footer="The color of your messages. Saved on this device."
    >
      {(Object.keys(bubblePalettes) as BubblePalette[]).map((key) => (
        <SettingsCheckRow
          key={key}
          label={bubblePalettes[key].label}
          selected={bubblePalette === key}
          onPress={() => setBubblePalette(key)}
          leading={
            <View
              style={[
                styles.swatch,
                {
                  backgroundColor: tone(key).background,
                  borderColor: colors.controlBorder,
                },
              ]}
            />
          }
        />
      ))}
    </SettingsGroup>
  );
}

const styles = StyleSheet.create({
  swatch: {
    width: 22,
    height: 22,
    borderRadius: 11,
    borderWidth: StyleSheet.hairlineWidth,
  },
});
