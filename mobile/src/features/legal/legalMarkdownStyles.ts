import type { TextStyle } from "react-native";
import type { ThemeColors } from "../../theme/mobileTheme";
import { radius, spacing, typeScale } from "../../theme/designTokens";

/** Inherit text colors and replace the renderer's fixed light surfaces. */
export function createLegalMarkdownStyles(c: ThemeColors) {
  const heading = { color: c.textPrimary, marginTop: spacing.lg, marginBottom: spacing.sm };
  const code = {
    ...typeScale.mono,
    backgroundColor: c.cardSoft,
    borderColor: c.border,
    borderRadius: radius.xs,
  };
  return {
    body: { color: c.textSecondary, ...typeScale.caption, lineHeight: 20 },
    heading1: { ...heading, ...typeScale.h2 },
    heading2: { ...heading, ...typeScale.bodyStrong },
    heading3: { ...heading, ...typeScale.bodyStrong, marginTop: spacing.md, marginBottom: spacing.xs },
    heading4: { color: c.textPrimary },
    heading5: { color: c.textPrimary },
    heading6: { color: c.textPrimary },
    paragraph: { color: c.textSecondary, ...typeScale.caption, lineHeight: 20, marginBottom: spacing.sm },
    strong: { color: c.textPrimary },
    bullet_list: { marginBottom: spacing.sm },
    list_item: { marginBottom: spacing.xs },
    blockquote: { backgroundColor: c.cardSoft, borderColor: c.border },
    code_inline: { ...code, color: c.textPrimary, paddingHorizontal: spacing.xs },
    code_block: code,
    fence: code,
    table: { borderColor: c.border },
    tr: { borderColor: c.border },
    // Underlines distinguish links from prose without relying on hue alone.
    link: { color: c.primary, textDecorationLine: "underline" },
    blocklink: { borderColor: c.primary },
    pre: { backgroundColor: c.cardSoft },
    hr: { backgroundColor: c.borderSoft, height: 1, marginVertical: spacing.md },
  } satisfies Record<string, TextStyle>;
}
