import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Keyboard,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { Check, Mic, Square, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { formatElapsed } from "./voice/dictationLevel";
import type { ComposerDictationState } from "./useComposerDictation";

const BAR_PITCH = 5;

export function ComposerDictationButton({
  dictation,
  disabled,
}: {
  dictation: ComposerDictationState;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  const held = useRef(false);
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
      accessibilityHint="Writes what you say into your message. Nothing is sent until you tap Send."
      accessibilityState={{ disabled: !!disabled || busy, busy }}
      disabled={disabled || busy}
      onLongPress={() => {
        if (!dictation.active) {
          held.current = true;
          Keyboard.dismiss();
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
        if (dictation.recording) dictation.stop();
        else {
          // Dictation runs in this composer; iOS asks for microphone and
          // speech recognition access itself the first time.
          Keyboard.dismiss();
          void dictation.start();
        }
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

/**
 * While dictation runs it lives inside the composer: the words appear above a
 * streaming waveform that takes the place of the message field, with Discard
 * and Finish. Finishing adds the words to the draft; nothing is sent. Idle
 * errors stay a compact notice above the still-usable text field.
 */
export function ComposerDictationReview({
  dictation,
  draft = "",
}: {
  dictation: ComposerDictationState;
  draft?: string;
}) {
  if (dictation.active) return <DictationInline dictation={dictation} draft={draft} />;
  if (!dictation.error) return null;
  return (
    <View style={styles.notice}>
      <Label accessibilityLiveRegion="polite" muted style={styles.noticeText}>
        {dictation.error}
      </Label>
      <View style={styles.noticeActions}>
        {dictation.permissionDenied && (
          <ActionButton
            label="Open Settings"
            compact
            variant="secondary"
            onPress={dictation.openSettings}
          />
        )}
        <ActionButton
          label="Dismiss"
          accessibilityLabel="Dismiss dictation message"
          compact
          variant="secondary"
          onPress={dictation.discard}
        />
      </View>
    </View>
  );
}

function DictationInline({
  dictation,
  draft,
}: {
  dictation: ComposerDictationState;
  draft: string;
}) {
  const { colors } = useTheme();
  const scroller = useRef<ScrollView>(null);
  const { phase } = dictation;
  const [startedAt, setStartedAt] = useState<number>();
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (phase !== "listening") return;
    setStartedAt((value) => value ?? Date.now());
    setNow(Date.now());
    const timer = setInterval(() => setNow(Date.now()), 500);
    return () => clearInterval(timer);
  }, [phase]);
  const capturing = phase === "listening" || phase === "stopping";
  const before = draft.trim() ? draft.replace(/\s+$/, "") : "";
  const spoken = !!dictation.text;
  const status =
    phase === "permission"
      ? "Waiting for microphone…"
      : phase === "connecting"
        ? "Starting…"
        : undefined;
  return (
    <View style={styles.inline}>
      {before || spoken ? (
        <ScrollView
          ref={scroller}
          style={styles.inlineText}
          contentContainerStyle={styles.inlineTextContent}
          // The newest words stay in view while speaking.
          onContentSizeChange={() =>
            scroller.current?.scrollToEnd?.({ animated: true })
          }
        >
          <Text
            accessibilityLiveRegion="polite"
            style={[styles.inlineBody, { color: colors.textPrimary }]}
          >
            {before ? (
              <Text style={{ color: colors.textSecondary }}>
                {before}
                {spoken ? " " : ""}
              </Text>
            ) : null}
            {dictation.final}
            {dictation.interim ? (
              <Text style={{ color: colors.textTertiary }}>
                {dictation.final ? " " : ""}
                {dictation.interim}
              </Text>
            ) : null}
          </Text>
        </ScrollView>
      ) : null}
      {dictation.error ? (
        <Label
          accessibilityLiveRegion="polite"
          style={[styles.inlineError, { color: colors.danger }]}
        >
          {dictation.error}
        </Label>
      ) : null}
      <View style={styles.inlineRow}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Discard dictation"
          onPress={dictation.discard}
          style={styles.target}
        >
          {({ pressed }) => (
            <View
              style={[
                styles.inlineCircle,
                {
                  borderColor: colors.controlBorder,
                  opacity: pressed ? 0.6 : 1,
                },
              ]}
            >
              <X size={17} color={colors.textPrimary} />
            </View>
          )}
        </Pressable>
        <View style={styles.inlineMeter}>
          {capturing ? (
            <DictationWaveform subscribe={dictation.subscribeLevel} height={22} />
          ) : (
            <View style={styles.inlineStatus}>
              <ActivityIndicator size="small" color={colors.textSecondary} />
              <Text
                accessibilityLiveRegion="polite"
                style={[styles.status, { color: colors.textSecondary }]}
              >
                {status}
              </Text>
            </View>
          )}
        </View>
        {startedAt !== undefined ? (
          <Text
            accessibilityLabel={`Recorded ${formatElapsed(now - startedAt)}`}
            style={[styles.elapsed, { color: colors.textTertiary }]}
          >
            {formatElapsed(now - startedAt)}
          </Text>
        ) : null}
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Finish dictation"
          accessibilityHint="Adds the words to your message. Nothing is sent until you tap Send."
          accessibilityState={{
            disabled: phase !== "listening",
            busy: phase === "stopping",
          }}
          disabled={phase !== "listening"}
          onPress={dictation.stop}
          style={styles.target}
        >
          {({ pressed }) => (
            <View
              style={[
                styles.inlineCircle,
                styles.inlineFinish,
                {
                  backgroundColor: colors.textPrimary,
                  opacity:
                    phase !== "listening" && phase !== "stopping"
                      ? 0.28
                      : pressed
                        ? 0.7
                        : 1,
                },
              ]}
            >
              {phase === "stopping" ? (
                <ActivityIndicator size="small" color={colors.bg} />
              ) : (
                <Check size={18} color={colors.bg} strokeWidth={2.5} />
              )}
            </View>
          )}
        </Pressable>
      </View>
    </View>
  );
}

/** One bar per recognizer volume sample (about 90ms), newest at the trailing edge. */
function DictationWaveform({
  subscribe,
  height,
}: {
  subscribe: ComposerDictationState["subscribeLevel"];
  height: number;
}) {
  const { colors } = useTheme();
  const [width, setWidth] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const count = Math.max(0, Math.floor(width / BAR_PITCH));
  const capacity = useRef(count);
  capacity.current = count;
  useEffect(
    () =>
      subscribe((level) =>
        setLevels((previous) =>
          [...previous, level].slice(-Math.max(1, capacity.current)),
        ),
      ),
    [subscribe],
  );
  const bars = [
    ...Array<number>(Math.max(0, count - levels.length)).fill(0),
    ...levels.slice(-count),
  ];
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
      style={[styles.wave, { height }]}
    >
      {bars.map((level, index) => (
        <View
          key={index}
          style={[
            styles.bar,
            {
              height: 3 + level * (height - 3),
              backgroundColor: colors.textSecondary,
            },
          ]}
        />
      ))}
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
  notice: { paddingHorizontal: 12, paddingTop: 8, gap: 6 },
  inline: { gap: 2 },
  inlineText: { maxHeight: 120 },
  inlineTextContent: { paddingHorizontal: 12, paddingTop: 12, paddingBottom: 4 },
  inlineBody: { fontFamily: fonts.bodyRegular, fontSize: 14, lineHeight: 20 },
  inlineError: { paddingHorizontal: 12, fontSize: 13 },
  inlineRow: { flexDirection: "row", alignItems: "center", minHeight: 44 },
  inlineMeter: { flex: 1, minWidth: 0, justifyContent: "center", paddingHorizontal: 4 },
  inlineStatus: { flexDirection: "row", alignItems: "center", gap: 8 },
  inlineCircle: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: "center",
    justifyContent: "center",
  },
  inlineFinish: { borderWidth: 0 },
  noticeText: { fontSize: 13 },
  noticeActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  wave: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: BAR_PITCH - 3,
    overflow: "hidden",
  },
  bar: { width: 3, borderRadius: 1.5 },
});
