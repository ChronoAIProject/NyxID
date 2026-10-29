import { useRef } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { Mic, Square } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { VoiceMediaBridge } from "./voice/VoiceMediaBridge";
import type { ComposerDictationState } from "./useComposerDictation";
import { OverflowScrollView } from "../../components/workspace/ScrollEdgeFades";

export function ComposerDictationButton({
  dictation,
  disabled,
}: {
  dictation: ComposerDictationState;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  const held = useRef(false);
  const suppressTap = useRef(false);
  const busy =
    dictation.phase === "permission" ||
    dictation.phase === "connecting" ||
    dictation.phase === "stopping";
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={
        dictation.recording ? "Stop dictation" : "Dictate message"
      }
      accessibilityHint="Transcribes speech for review before inserting it into your message."
      accessibilityState={{
        disabled: !!disabled || busy || dictation.phase === "review",
        busy,
      }}
      disabled={disabled || busy || dictation.phase === "review"}
      onLongPress={() => {
        if (!dictation.active) {
          held.current = true;
          suppressTap.current = true;
          void dictation.start();
        }
      }}
      onPressOut={() => {
        if (held.current) {
          held.current = false;
          dictation.stop();
        }
      }}
      onPress={() => {
        if (suppressTap.current) {
          suppressTap.current = false;
          return;
        }
        if (dictation.recording) dictation.stop();
        else void dictation.start();
      }}
      style={({ pressed }) => [
        styles.target,
        { opacity: disabled || busy ? 0.4 : pressed ? 0.65 : 1 },
      ]}
    >
      {busy ? (
        <ActivityIndicator size="small" color={colors.textSecondary} />
      ) : dictation.recording ? (
        <Square size={17} color={colors.danger} fill={colors.danger} />
      ) : (
        <Mic size={21} color={colors.textPrimary} />
      )}
    </Pressable>
  );
}

export function ComposerDictationReview({
  dictation,
}: {
  dictation: ComposerDictationState;
}) {
  const { colors } = useTheme();
  if (!dictation.active && !dictation.error) return null;
  return (
    <View style={styles.review}>
      {dictation.media && dictation.origin && (
        <VoiceMediaBridge media={dictation.media} origin={dictation.origin} />
      )}
      <Label accessibilityLiveRegion="polite" muted>
        {dictation.error ??
          (dictation.phase === "permission"
            ? "Waiting for microphone permission…"
            : dictation.phase === "connecting"
              ? "Connecting dictation…"
              : dictation.phase === "stopping"
                ? "Finishing dictation…"
                : dictation.phase === "review"
                  ? "Review dictation"
                  : "Listening…")}
      </Label>
      {!!dictation.text && (
        <OverflowScrollView
          style={styles.transcript}
          fadeColor={colors.composerBg}
        >
          <Label>{dictation.text}</Label>
        </OverflowScrollView>
      )}
      <View style={styles.actions}>
        {dictation.permissionDenied && (
          <ActionButton
            label="Open Settings"
            variant="secondary"
            onPress={dictation.openSettings}
          />
        )}
        {dictation.phase === "review" && (
          <ActionButton label="Insert text" onPress={dictation.insert} />
        )}
        <ActionButton
          label={
            dictation.error ? "Dismiss dictation message" : "Discard dictation"
          }
          variant="secondary"
          onPress={dictation.discard}
        />
      </View>
    </View>
  );
}

const styles = StyleSheet.create({
  target: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  review: { paddingHorizontal: 12, paddingTop: 8, gap: 6 },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  transcript: { maxHeight: 120 },
});
