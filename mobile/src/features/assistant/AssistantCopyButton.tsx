import { useEffect, useRef, useState } from "react";
import { Alert, Pressable, StyleSheet, View } from "react-native";
import * as Clipboard from "expo-clipboard";
import { Check, Copy } from "lucide-react-native";
import { useTheme } from "../../theme/ThemeContext";

/** Copy the exact message body, preserving links, Markdown and code. */
export function AssistantCopyButton({
  text,
  partial = false,
  afterDate = false,
  foreground,
}: {
  text: string;
  partial?: boolean;
  afterDate?: boolean;
  foreground?: string;
}) {
  const { colors } = useTheme();
  const [copiedText, setCopiedText] = useState<string>();
  const [copying, setCopying] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (copiedText === undefined) return;
    const timer = setTimeout(() => setCopiedText(undefined), 2000);
    return () => clearTimeout(timer);
  }, [copiedText]);
  if (!text.trim()) return null;
  const copied = copiedText === text;
  const label = copied
    ? "Copied"
    : partial
      ? "Copy response so far"
      : "Copy message";
  async function copy() {
    if (copying) return;
    setCopying(true);
    try {
      const success = await Clipboard.setStringAsync(text);
      if (!success) throw new Error("Clipboard unavailable");
      if (mounted.current) setCopiedText(text);
    } catch {
      if (mounted.current)
        Alert.alert("Couldn't copy message", "Please try again.");
    } finally {
      if (mounted.current) setCopying(false);
    }
  }
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityHint={
        partial
          ? "Copy the response text received so far to the clipboard"
          : "Copy this message to the clipboard"
      }
      accessibilityState={{ disabled: copying }}
      disabled={copying}
      // Split the horizontal expansion at half the 8pt gap before the date.
      hitSlop={{
        top: 10,
        bottom: 10,
        left: afterDate ? 4 : 16,
        right: afterDate ? 16 : 4,
      }}
      onPress={() => void copy()}
      style={({ pressed }) => [
        styles.button,
        { opacity: pressed || copying ? 0.55 : 1 },
      ]}
    >
      <View
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        {copied ? (
          <Check size={16} color={foreground ?? colors.textMuted} />
        ) : (
          <Copy size={16} color={foreground ?? colors.textMuted} />
        )}
      </View>
    </Pressable>
  );
}

const styles = StyleSheet.create({
  button: {
    width: 24,
    height: 24,
    padding: 4,
    alignSelf: "flex-start",
    flexDirection: "row",
    alignItems: "center",
  },
});
