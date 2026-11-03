import { useEffect, useRef, useState } from "react";
import { Alert, Pressable, StyleSheet, View } from "react-native";
import { Flag } from "lucide-react-native";
import { useTheme } from "../../theme/ThemeContext";
import { replyReportApi } from "../../lib/api/replyReportApi";
import {
  REPORT_REASONS,
  type ReplyReportTarget,
  type ReportReason,
} from "../../lib/api/replyReportTransport";

/** Flag an agent reply as offensive, harmful or wrong for NyxID review. */
export function AssistantReportButton({
  target,
  reported: initiallyReported = false,
}: {
  target: ReplyReportTarget;
  reported?: boolean;
}) {
  const { colors } = useTheme();
  const [reported, setReported] = useState(initiallyReported);
  const [sending, setSending] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  useEffect(() => {
    if (initiallyReported) setReported(true);
  }, [initiallyReported]);

  async function send(reason: ReportReason) {
    setSending(true);
    try {
      await replyReportApi.report(target, reason);
      if (!mounted.current) return;
      setReported(true);
      Alert.alert(
        "Report sent",
        "Thanks for letting us know. The NyxID team reviews every report.",
      );
    } catch {
      if (mounted.current)
        Alert.alert(
          "Couldn't send report",
          "Check your connection and try again, or email contact@chrono-ai.fun.",
        );
    } finally {
      if (mounted.current) setSending(false);
    }
  }

  function choose() {
    if (sending) return;
    Alert.alert(
      reported ? "Report this response again?" : "Report this response?",
      "Choose what is wrong. The NyxID team reviews every report.",
      [
        ...REPORT_REASONS.map((reason) => ({
          text: reason,
          onPress: () => void send(reason),
        })),
        { text: "Cancel", style: "cancel" as const },
      ],
    );
  }

  const tint = reported ? colors.danger : colors.textMuted;
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={reported ? "Reported response" : "Report response"}
      accessibilityHint="Flag this response as offensive, harmful or inaccurate"
      accessibilityState={{ disabled: sending }}
      disabled={sending}
      hitSlop={{ top: 10, bottom: 10, left: 4, right: 16 }}
      onPress={choose}
      style={({ pressed }) => [
        styles.button,
        { opacity: pressed || sending ? 0.55 : 1 },
      ]}
    >
      <View
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
      >
        <Flag size={16} color={tint} fill={reported ? tint : "none"} />
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
