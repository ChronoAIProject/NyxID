import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Keyboard,
  Pressable,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { Check, Mic, Square, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  VoiceMediaBridge,
  type NativeVoiceMedia,
} from "./voice/VoiceMediaBridge";
import { formatElapsed, pcmLevel } from "./voice/dictationLevel";
import type { ComposerDictationState } from "./useComposerDictation";
import { OverflowScrollView } from "../../components/workspace/ScrollEdgeFades";

const BAR_PITCH = 5;
const BAR_INTERVAL_MS = 90;

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
      accessibilityHint="Transcribes speech for review before inserting it into your message."
      accessibilityState={{
        disabled: !!disabled || busy || dictation.phase === "review",
        busy,
      }}
      disabled={disabled || busy || dictation.phase === "review"}
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
 * While dictation runs it takes over the composer: the transcript is the
 * focus, with a live level meter and one trailing Done/Insert action. Idle
 * errors stay a compact notice above the still-usable text field.
 */
export function ComposerDictationReview({
  dictation,
}: {
  dictation: ComposerDictationState;
}) {
  if (dictation.active) return <DictationPanel dictation={dictation} />;
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

function DictationPanel({ dictation }: { dictation: ComposerDictationState }) {
  const { colors } = useTheme();
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
  const status =
    phase === "permission"
      ? "Waiting for microphone…"
      : phase === "connecting"
        ? "Connecting…"
        : phase === "stopping"
          ? "Finishing…"
          : phase === "review"
            ? "Review before inserting"
            : "Listening";
  const capturing = phase === "listening" || phase === "stopping";
  return (
    <View style={styles.panel}>
      {dictation.media && dictation.origin && (
        <VoiceMediaBridge media={dictation.media} origin={dictation.origin} />
      )}
      <View style={styles.statusRow}>
        {phase === "listening" ? (
          <View style={[styles.liveDot, { backgroundColor: colors.danger }]} />
        ) : phase === "review" ? null : (
          <ActivityIndicator size="small" color={colors.textSecondary} />
        )}
        <Text
          accessibilityLiveRegion="polite"
          style={[styles.status, { color: colors.textSecondary }]}
        >
          {status}
        </Text>
        {startedAt !== undefined && (
          <Text
            accessibilityLabel={`Recorded ${formatElapsed(now - startedAt)}`}
            style={[styles.elapsed, { color: colors.textTertiary }]}
          >
            {formatElapsed(now - startedAt)}
          </Text>
        )}
      </View>
      {dictation.text ? (
        <OverflowScrollView
          style={styles.transcript}
          fadeColor={colors.composerBg}
        >
          <Text style={[styles.transcriptText, { color: colors.textPrimary }]}>
            {dictation.final}
            {dictation.interim ? (
              <Text style={{ color: colors.textTertiary }}>
                {dictation.final ? " " : ""}
                {dictation.interim}
              </Text>
            ) : null}
          </Text>
        </OverflowScrollView>
      ) : phase === "listening" ? (
        <Text style={[styles.transcriptText, { color: colors.textTertiary }]}>
          Start speaking…
        </Text>
      ) : null}
      {dictation.error ? (
        <Label
          accessibilityLiveRegion="polite"
          style={{ color: colors.danger }}
        >
          {dictation.error}
        </Label>
      ) : null}
      <View style={styles.controls}>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Discard dictation"
          onPress={dictation.discard}
          style={styles.target}
        >
          {({ pressed }) => (
            <View
              style={[
                styles.discard,
                {
                  borderColor: colors.controlBorder,
                  opacity: pressed ? 0.65 : 1,
                },
              ]}
            >
              <X size={18} color={colors.textPrimary} />
            </View>
          )}
        </Pressable>
        <View style={styles.meter}>
          {capturing && <DictationWaveform media={dictation.media} />}
        </View>
        {phase === "review" ? (
          <PillButton
            label="Insert"
            accessibilityLabel="Insert text"
            icon={<Check size={16} color={colors.bg} strokeWidth={2.5} />}
            onPress={dictation.insert}
          />
        ) : (
          <PillButton
            label="Done"
            accessibilityLabel="Finish dictation"
            icon={<Square size={10} color={colors.bg} fill={colors.bg} />}
            busy={phase === "stopping"}
            disabled={phase !== "listening"}
            onPress={dictation.stop}
          />
        )}
      </View>
    </View>
  );
}

function PillButton({
  label,
  accessibilityLabel,
  icon,
  busy,
  disabled,
  onPress,
}: {
  label: string;
  accessibilityLabel: string;
  icon: ReactNode;
  busy?: boolean;
  disabled?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={accessibilityLabel}
      accessibilityState={{ disabled: !!disabled, busy: !!busy }}
      disabled={disabled || busy}
      onPress={onPress}
      style={styles.pillTarget}
    >
      {({ pressed }) => (
        <View
          style={[
            styles.pill,
            {
              backgroundColor: colors.textPrimary,
              opacity: disabled && !busy ? 0.28 : pressed ? 0.7 : 1,
            },
          ]}
        >
          {busy ? <ActivityIndicator size="small" color={colors.bg} /> : icon}
          <Text style={[styles.pillLabel, { color: colors.bg }]}>{label}</Text>
        </View>
      )}
    </Pressable>
  );
}

/** Peak microphone level per 90ms, newest bar at the trailing edge. */
function DictationWaveform({ media }: { media: NativeVoiceMedia | null }) {
  const { colors } = useTheme();
  const [width, setWidth] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const count = Math.max(0, Math.floor(width / BAR_PITCH));
  const capacity = useRef(count);
  capacity.current = count;
  useEffect(() => {
    if (!media) return;
    let peak = 0;
    const unsubscribe = media.subscribe((event) => {
      if (event.type === "pcm") peak = Math.max(peak, pcmLevel(event.bytes));
    });
    const timer = setInterval(() => {
      const value = peak;
      peak = 0;
      setLevels((previous) =>
        [...previous, value].slice(-Math.max(1, capacity.current)),
      );
    }, BAR_INTERVAL_MS);
    return () => {
      unsubscribe();
      clearInterval(timer);
    };
  }, [media]);
  const bars = [
    ...Array<number>(Math.max(0, count - levels.length)).fill(0),
    ...levels.slice(-count),
  ];
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
      style={styles.wave}
    >
      {bars.map((level, index) => (
        <View
          key={index}
          style={[
            styles.bar,
            {
              height: 3 + level * 21,
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
  noticeText: { fontSize: 13 },
  noticeActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  panel: { paddingHorizontal: 12, paddingTop: 10, gap: 8 },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 20,
  },
  liveDot: { width: 8, height: 8, borderRadius: 4 },
  status: { flex: 1, fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  transcript: { maxHeight: 168 },
  transcriptText: {
    fontFamily: fonts.bodyRegular,
    fontSize: 17,
    lineHeight: 24,
  },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    minHeight: 44,
    marginHorizontal: -8,
  },
  discard: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: 1,
    alignItems: "center",
    justifyContent: "center",
  },
  meter: { flex: 1, height: 24, marginHorizontal: 4 },
  wave: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    gap: BAR_PITCH - 3,
    overflow: "hidden",
  },
  bar: { width: 3, borderRadius: 1.5 },
  pillTarget: { minHeight: 44, justifyContent: "center", paddingHorizontal: 4 },
  pill: {
    height: 34,
    borderRadius: 17,
    paddingHorizontal: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  pillLabel: { fontFamily: fonts.bodySemi, fontSize: 14 },
});
