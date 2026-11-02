import { useState } from "react";
import { Alert, Pressable, StyleSheet, Text, View } from "react-native";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { AssistantCopyButton } from "./AssistantCopyButton";
import { messageTime } from "./messageTime";

export function AssistantMessageTime({
  value,
  beforeCopy = false,
  foreground,
}: {
  value?: string | number | null;
  beforeCopy?: boolean;
  foreground?: string;
}) {
  const { colors } = useTheme();
  const [size, setSize] = useState({ width: 0, height: 0 });
  const time = messageTime(value);
  if (!time) return null;
  const horizontal = Math.max(0, 44 - size.width);
  const adjacent = Math.min(4, horizontal / 2);
  const vertical = Math.max(0, (44 - size.height) / 2);
  return (
    <Pressable
      accessibilityRole="text"
      accessibilityLabel={time.label}
      accessibilityHint="Touch and hold for the full date and time"
      onLongPress={() => Alert.alert("Message sent", time.label)}
      hitSlop={{
        top: vertical,
        bottom: vertical,
        left: beforeCopy ? horizontal - adjacent : adjacent,
        right: beforeCopy ? adjacent : horizontal - adjacent,
      }}
      onLayout={({ nativeEvent: { layout } }) => {
        if (layout.width !== size.width || layout.height !== size.height)
          setSize({ width: layout.width, height: layout.height });
      }}
      style={styles.timeTarget}
    >
      <Text style={[styles.time, { color: foreground ?? colors.textMuted }]}>
        {time.short}
      </Text>
    </Pressable>
  );
}

/** Native replies intentionally offer Copy and date, without reply voting. */
export function AssistantMessageFooter({
  text,
  timestamp,
  user,
  foreground,
}: {
  text: string;
  timestamp?: string | number | null;
  user?: boolean;
  foreground?: string;
}) {
  const copy = <AssistantCopyButton text={text} afterDate={user} foreground={foreground} />;
  const time = <AssistantMessageTime value={timestamp} beforeCopy={user} foreground={foreground} />;
  return (
    <View style={[styles.row, user && styles.userRow]}>
      {user ? time : copy}
      {user ? copy : time}
    </View>
  );
}
const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    alignItems: "center",
    flexWrap: "wrap",
    gap: 8,
    marginTop: 8,
  },
  userRow: { justifyContent: "flex-end" },
  timeTarget: {
    justifyContent: "center",
  },
  time: { fontFamily: fonts.bodyRegular, fontSize: 11 },
});
