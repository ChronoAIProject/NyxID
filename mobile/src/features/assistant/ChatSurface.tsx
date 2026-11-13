import type { ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { useTheme } from "../../theme/ThemeContext";
import {
  chatBubbleColors,
  userBubbleCorners,
} from "../../theme/chatAppearance";
import { fonts, radius } from "../../theme/designTokens";
import { useChatColumn } from "../display/ChatLayoutContext";

/** Private and crew pages use the same transcript rhythm and composer grid. */
export const chatSurfaceStyles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0, minWidth: 0 },
  row: { flexDirection: "row", alignItems: "center", gap: 8 },
  iconButton: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: radius.md,
  },
  transcript: { paddingVertical: 8, flexGrow: 1 },
  transcriptHeader: { marginBottom: 20 },
  transcriptSeparator: { height: 20 },
  message: { gap: 8, paddingVertical: 10 },
  userTurn: { gap: 8 },
  userRow: {
    alignSelf: "flex-end",
    alignItems: "flex-end",
    marginLeft: 38,
    gap: 2,
    paddingVertical: 6,
  },
  userMessage: {
    paddingHorizontal: 14,
    paddingVertical: 10,
    ...userBubbleCorners,
  },
  messageLabel: { fontSize: 11, fontFamily: fonts.bodySemi },
  messageText: { fontFamily: fonts.bodyRegular, fontSize: 14, lineHeight: 23 },
  footer: { gap: 16, paddingTop: 20 },
  composerInset: { paddingHorizontal: 12 },
  chatPage: { paddingHorizontal: 0, paddingTop: 0, paddingBottom: 4, gap: 8 },
  composer: { marginBottom: 0 },
  composerHint: { fontSize: 11, lineHeight: 16, paddingHorizontal: 12 },
  errorRow: { gap: 8, marginHorizontal: 12 },
});

/** Transcript notes and author metadata stay outside a user's coloured bubble. */
export function ChatMessageTurn({
  user,
  notes,
  author,
  footer,
  children,
}: {
  user?: boolean;
  notes?: ReactNode;
  author?: ReactNode;
  footer?: ReactNode;
  children: ReactNode;
}) {
  const { mode, bubblePalette } = useTheme();
  const bubble = chatBubbleColors(mode, bubblePalette);
  if (user)
    return (
      <View style={chatSurfaceStyles.userTurn}>
        {notes}
        <View style={chatSurfaceStyles.userRow}>
          {author}
          <View
            style={[
              chatSurfaceStyles.message,
              chatSurfaceStyles.userMessage,
              { backgroundColor: bubble.background },
            ]}
          >
            {children}
          </View>
          {footer}
        </View>
      </View>
    );
  return (
    <View style={chatSurfaceStyles.message}>
      {notes}
      {author}
      {children}
      {footer}
    </View>
  );
}

export function ChatTranscriptSeparator() {
  return <View style={chatSurfaceStyles.transcriptSeparator} />;
}

/** Text and voice composers occupy precisely the same inset and saved column. */
export function ChatComposerDock({ children }: { children: ReactNode }) {
  const column = useChatColumn();
  return (
    <View style={chatSurfaceStyles.composerInset}>
      <View style={[chatSurfaceStyles.composer, column]}>{children}</View>
    </View>
  );
}

export function ChatIconButton({
  children,
  label,
  onPress,
  disabled,
}: {
  children: ReactNode;
  label: string;
  onPress: () => void;
  disabled?: boolean;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        chatSurfaceStyles.iconButton,
        { opacity: disabled ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      {children}
    </Pressable>
  );
}
