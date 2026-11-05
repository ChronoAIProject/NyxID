import { StyleSheet, View } from "react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useTheme } from "../../theme/ThemeContext";
import { darkColors, lightColors } from "../../theme/mobileTheme";
import {
  chatBubbleColors,
  userBubbleCorners,
  type BubblePalette,
} from "../../theme/chatAppearance";
import { spacing, typeScale } from "../../theme/designTokens";

/**
 * A small sample conversation drawn with the chosen appearance, so every
 * Display choice shows its effect before you leave the page.
 */
export function ChatPreview({
  mode: modeProp,
  palette: paletteProp,
  composerHeight,
  columnRatio,
  compact = false,
}: {
  mode?: "light" | "dark";
  palette?: BubblePalette;
  /** Composer height in pt, drawn at half scale. */
  composerHeight?: number;
  /** Share of the preview's width the chat column fills (0..1). */
  columnRatio?: number;
  /** Thumbnail size for the Light/Dark appearance tiles. */
  compact?: boolean;
}) {
  const theme = useTheme();
  const mode = modeProp ?? (theme.mode === "dark" ? "dark" : "light");
  const palette = paletteProp ?? theme.bubblePalette;
  const colors = mode === "dark" ? darkColors : lightColors;
  const bubble = chatBubbleColors(mode, palette);
  const text = compact ? styles.compactText : styles.text;
  return (
    <View
      accessible
      accessibilityLabel={`Chat preview, ${mode} appearance`}
      style={[
        styles.frame,
        compact && styles.compactFrame,
        { backgroundColor: colors.bg, borderColor: colors.borderSoft },
      ]}
    >
      <View
        style={[
          styles.column,
          columnRatio !== undefined && {
            width: `${Math.round(Math.max(0.35, Math.min(1, columnRatio)) * 100)}%`,
          },
        ]}
      >
        <View
          style={[
            styles.bubble,
            compact && styles.compactBubble,
            { backgroundColor: bubble.background },
          ]}
        >
          <Text style={[text, { color: bubble.foreground }]}>
            {compact ? "Hi NyxBot" : "Show me my connected services."}
          </Text>
        </View>
        <Text style={[text, { color: colors.textPrimary }]}>
          {compact
            ? "Here you go."
            : "You have GitHub, Gmail and Notion connected."}
        </Text>
        <View
          style={[
            styles.composer,
            compact && styles.compactComposer,
            {
              backgroundColor: colors.composerBg,
              borderColor: colors.composerBorder,
            },
            composerHeight !== undefined && {
              height: Math.round(composerHeight / 2),
            },
          ]}
        >
          {compact ? null : (
            <Text style={[styles.placeholder, { color: colors.textMuted }]}>
              Message NyxBot
            </Text>
          )}
        </View>
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  frame: {
    borderRadius: 16,
    borderWidth: StyleSheet.hairlineWidth,
    padding: spacing.lg,
    alignItems: "center",
  },
  compactFrame: { borderRadius: 12, padding: spacing.sm },
  column: { width: "100%", gap: spacing.sm },
  bubble: {
    ...userBubbleCorners,
    alignSelf: "flex-end",
    maxWidth: "80%",
    paddingVertical: 8,
    paddingHorizontal: 12,
  },
  compactBubble: { paddingVertical: 3, paddingHorizontal: 6 },
  text: { ...typeScale.description },
  compactText: { fontSize: 9, lineHeight: 12 },
  composer: {
    marginTop: spacing.sm,
    minHeight: 30,
    borderRadius: 14,
    borderWidth: 1,
    paddingHorizontal: spacing.md,
    paddingVertical: 6,
  },
  compactComposer: { minHeight: 12, borderRadius: 6, marginTop: 2 },
  placeholder: { ...typeScale.small },
});
