import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Keyboard,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { Check, Globe, Mic, ShieldCheck, Square, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  VoiceMediaBridge,
  type NativeVoiceMedia,
} from "./voice/VoiceMediaBridge";
import { formatElapsed, pcmLevel } from "./voice/dictationLevel";
import type { ComposerDictationState } from "./useComposerDictation";

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
  const [intro, setIntro] = useState(false);
  const busy =
    dictation.phase === "permission" ||
    dictation.phase === "connecting" ||
    dictation.phase === "stopping";
  return (
    <>
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
            // The first tap explains dictation once; a held press is already a
            // deliberate push-to-talk and starts at once.
            void introSeen().then((seen) => {
              if (seen) void dictation.start();
              else setIntro(true);
            });
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
      <DictationIntro
        visible={intro}
        onClose={() => setIntro(false)}
        onStart={() => {
          setIntro(false);
          void AsyncStorage.setItem(INTRO_KEY, "1").catch(() => {});
          void dictation.start();
        }}
      />
    </>
  );
}

const INTRO_KEY = "nyxid.dictation_intro_seen";
async function introSeen() {
  try {
    return (await AsyncStorage.getItem(INTRO_KEY)) === "1";
  } catch {
    return true;
  }
}

/**
 * One-time explanation as a compact sheet over the chat (the mockup's
 * "Speech to Text" card): what it does, microphone access, language, Start.
 */
function DictationIntro({
  visible,
  onClose,
  onStart,
}: {
  visible: boolean;
  onClose: () => void;
  onStart: () => void;
}) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const rows = [
    {
      Icon: ShieldCheck,
      title: "Microphone access",
      detail:
        "iOS asks the first time you start. Recording stops when you tap Finish.",
    },
    {
      Icon: Globe,
      title: "Speech language",
      detail: "Detected automatically from what you say.",
    },
  ];
  return (
    <Modal
      visible={visible}
      transparent
      animationType="slide"
      onRequestClose={onClose}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel="Dismiss"
        onPress={onClose}
        style={[styles.introBackdrop, { backgroundColor: colors.sheetScrim }]}
      />
      <View
        accessibilityViewIsModal
        style={[
          styles.intro,
          {
            backgroundColor: colors.bg,
            paddingBottom: Math.max(insets.bottom, 16) + 12,
          },
        ]}
      >
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Close"
          onPress={onClose}
          hitSlop={8}
          style={[styles.introClose, { backgroundColor: colors.navActive }]}
        >
          <X size={18} color={colors.textPrimary} />
        </Pressable>
        <View style={styles.introHead}>
          <Mic size={30} color={colors.primary} />
          <Text
            accessibilityRole="header"
            style={[styles.introTitle, { color: colors.textPrimary }]}
          >
            Speech to Text
          </Text>
          <Text style={[styles.introLead, { color: colors.textSecondary }]}>
            Speak instead of typing. You review the text before it goes into
            your message, and nothing is sent on its own.
          </Text>
        </View>
        <View
          style={[styles.introCard, { backgroundColor: colors.groupedFill }]}
        >
          {rows.map(({ Icon, title, detail }, index) => (
            <View
              key={title}
              style={[
                styles.introRow,
                index > 0 && {
                  borderTopWidth: StyleSheet.hairlineWidth,
                  borderTopColor: colors.borderSoft,
                },
              ]}
            >
              <Icon size={20} color={colors.textSecondary} />
              <View style={{ flex: 1, gap: 2 }}>
                <Text
                  style={[styles.introRowTitle, { color: colors.textPrimary }]}
                >
                  {title}
                </Text>
                <Text
                  style={[
                    styles.introRowDetail,
                    { color: colors.textSecondary },
                  ]}
                >
                  {detail}
                </Text>
              </View>
            </View>
          ))}
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel="Start dictation"
          onPress={onStart}
          style={({ pressed }) => [
            styles.introStart,
            {
              backgroundColor: colors.textPrimary,
              opacity: pressed ? 0.75 : 1,
            },
          ]}
        >
          <Text style={[styles.introStartLabel, { color: colors.bg }]}>
            Start Dictation
          </Text>
        </Pressable>
      </View>
    </Modal>
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

/**
 * Dictation in progress fills the screen, as in the mockup: the transcript is
 * the page, set large and following the speech, with the live waveform and
 * one Finish (then Insert) action at the bottom. Nothing is sent from here.
 */
function DictationPanel({ dictation }: { dictation: ComposerDictationState }) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
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
    <Modal
      visible
      animationType="slide"
      presentationStyle="fullScreen"
      onRequestClose={dictation.discard}
    >
      <View
        style={[
          styles.panel,
          {
            backgroundColor: colors.bg,
            paddingTop: insets.top + 4,
            paddingBottom: Math.max(insets.bottom, 16),
          },
        ]}
      >
        {dictation.media && dictation.origin && (
          <VoiceMediaBridge media={dictation.media} origin={dictation.origin} />
        )}
        <View style={styles.panelHeader}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Discard dictation"
            onPress={dictation.discard}
            style={styles.target}
          >
            {({ pressed }) => (
              <X
                size={22}
                color={colors.textPrimary}
                opacity={pressed ? 0.6 : 1}
              />
            )}
          </Pressable>
          <View style={styles.panelTitle}>
            <Text
              accessibilityRole="header"
              style={[styles.panelTitleText, { color: colors.textPrimary }]}
            >
              Speech to Text
            </Text>
            <View style={styles.statusRow}>
              {phase === "listening" ? (
                <View
                  style={[styles.liveDot, { backgroundColor: colors.danger }]}
                />
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
          </View>
          <View style={styles.target} />
        </View>
        <ScrollView
          ref={scroller}
          style={styles.transcript}
          contentContainerStyle={styles.transcriptContent}
          // The newest words stay in view while speaking.
          onContentSizeChange={() => {
            if (phase !== "review") scroller.current?.scrollToEnd?.({ animated: true });
          }}
        >
          {dictation.text ? (
            <Text
              selectable={phase === "review"}
              style={[styles.transcriptText, { color: colors.textPrimary }]}
            >
              {dictation.final}
              {dictation.interim ? (
                <Text style={{ color: colors.textTertiary }}>
                  {dictation.final ? " " : ""}
                  {dictation.interim}
                </Text>
              ) : null}
            </Text>
          ) : (
            <Text
              style={[styles.transcriptText, { color: colors.textTertiary }]}
            >
              {phase === "listening" ? "Start speaking…" : ""}
            </Text>
          )}
        </ScrollView>
        {dictation.error ? (
          <Label
            accessibilityLiveRegion="polite"
            style={[styles.panelError, { color: colors.danger }]}
          >
            {dictation.error}
          </Label>
        ) : null}
        <View style={[styles.meter, { backgroundColor: colors.groupedFill }]}>
          {capturing && <DictationWaveform media={dictation.media} />}
        </View>
        <View style={styles.controls}>
          {phase === "review" ? (
            <PillButton
              label="Insert"
              accessibilityLabel="Insert text"
              icon={<Check size={16} color={colors.bg} strokeWidth={2.5} />}
              onPress={dictation.insert}
            />
          ) : (
            <PillButton
              label="Finish"
              accessibilityLabel="Finish dictation"
              icon={<Square size={10} color={colors.bg} fill={colors.bg} />}
              busy={phase === "stopping"}
              disabled={phase !== "listening"}
              onPress={dictation.stop}
            />
          )}
        </View>
        <Text style={[styles.footnote, { color: colors.textTertiary }]}>
          {phase === "review"
            ? "Edit after inserting · Nothing is sent until you tap Send"
            : "Language detected automatically"}
        </Text>
      </View>
    </Modal>
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
              height: 3 + level * 33,
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
  panel: { flex: 1, paddingHorizontal: 20, gap: 14 },
  panelHeader: {
    flexDirection: "row",
    alignItems: "center",
    marginHorizontal: -10,
  },
  panelTitle: { flex: 1, alignItems: "center", gap: 2 },
  panelTitleText: { fontFamily: fonts.displaySemi, fontSize: 17, lineHeight: 22 },
  panelError: { textAlign: "center" },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 20,
  },
  liveDot: { width: 8, height: 8, borderRadius: 4 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  // The transcript is the page: set large, as in a full-screen reading view.
  transcript: { flex: 1 },
  transcriptContent: { paddingTop: 12, paddingBottom: 24 },
  transcriptText: {
    fontFamily: fonts.displaySemi,
    fontSize: 30,
    lineHeight: 40,
  },
  footnote: {
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 16,
    textAlign: "center",
  },
  introBackdrop: { flex: 1 },
  intro: {
    borderTopLeftRadius: 28,
    borderTopRightRadius: 28,
    paddingHorizontal: 24,
    paddingTop: 16,
    gap: 20,
  },
  introClose: {
    alignSelf: "flex-end",
    width: 32,
    height: 32,
    borderRadius: 16,
    alignItems: "center",
    justifyContent: "center",
  },
  introHead: { alignItems: "center", gap: 10, paddingHorizontal: 8 },
  introTitle: { fontFamily: fonts.displaySemi, fontSize: 26, lineHeight: 32 },
  introLead: {
    fontFamily: fonts.bodyRegular,
    fontSize: 15,
    lineHeight: 21,
    textAlign: "center",
  },
  introCard: { borderRadius: 14, overflow: "hidden" },
  introRow: {
    flexDirection: "row",
    gap: 14,
    paddingHorizontal: 16,
    paddingVertical: 14,
    alignItems: "flex-start",
  },
  introRowTitle: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 20 },
  introRowDetail: {
    fontFamily: fonts.bodyRegular,
    fontSize: 13,
    lineHeight: 18,
  },
  introStart: {
    minHeight: 52,
    borderRadius: 26,
    alignItems: "center",
    justifyContent: "center",
  },
  introStartLabel: { fontFamily: fonts.bodySemi, fontSize: 16 },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    minHeight: 52,
  },
  meter: {
    height: 64,
    borderRadius: 20,
    paddingHorizontal: 14,
    justifyContent: "center",
  },
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
    height: 50,
    borderRadius: 25,
    paddingHorizontal: 30,
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
  },
  pillLabel: { fontFamily: fonts.bodySemi, fontSize: 15 },
});
