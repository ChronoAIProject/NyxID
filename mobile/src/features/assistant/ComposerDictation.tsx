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
import { Check, Globe, Mic, Square, X } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { LinearGradient } from "expo-linear-gradient";
import { ExpoSpeechRecognitionModule as Recognizer } from "expo-speech-recognition";
import { usePopupMenu } from "../../components/settings/SettingsList";
import { formatElapsed } from "./voice/dictationLevel";
import { dictationLocale } from "./voice/dictationLocale";
import {
  languageLabel,
  languageOptions,
  saveDictationLanguage,
  savedDictationLanguage,
} from "./voice/dictationLanguage";
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
  const [intro, setIntro] = useState(false);
  const language = useDictationLanguage(intro);
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
            Keyboard.dismiss();
            // Once speech recognition is allowed, a tap dictates straight into
            // the composer. Before that, the first tap explains dictation once;
            // a held press is already a deliberate push-to-talk and starts at once.
            void readyToDictate().then((ready) => {
              if (ready) void dictation.start();
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
        language={language}
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
async function readyToDictate() {
  try {
    const permission = await Recognizer.getPermissionsAsync();
    if (permission.granted) return true;
  } catch {
    // Fall back to the intro flag; start() still asks for permission.
  }
  try {
    return (await AsyncStorage.getItem(INTRO_KEY)) === "1";
  } catch {
    return true;
  }
}

/**
 * The dictation language: the person's choice when the recognizer supports
 * it, else the iPhone's. Changing it applies from the next recording.
 */
function useDictationLanguage(active: boolean) {
  const [saved, setSaved] = useState<string>();
  const [supported, setSupported] = useState<readonly string[]>([]);
  useEffect(() => {
    if (!active) return;
    let live = true;
    void savedDictationLanguage().then((value) => {
      if (live) setSaved(value);
    });
    void Recognizer.getSupportedLocales({})
      .then((result) => {
        if (live) setSupported(result.locales);
      })
      .catch(() => {});
    return () => {
      live = false;
    };
  }, [active]);
  let phone: string | undefined;
  try {
    phone = Intl.DateTimeFormat().resolvedOptions().locale;
  } catch {
    phone = undefined;
  }
  const automatic = phone ? dictationLocale(phone, supported) ?? phone : undefined;
  const current =
    saved && (!supported.length || supported.includes(saved)) ? saved : automatic;
  return {
    value: saved ?? AUTOMATIC,
    label: current ? languageLabel(current) : "Your iPhone's language",
    options: [
      {
        value: AUTOMATIC,
        label: "iPhone language",
        detail: automatic ? languageLabel(automatic) : undefined,
      },
      ...languageOptions(supported),
    ],
    choose: (value: string) => {
      const next = value === AUTOMATIC ? undefined : value;
      setSaved(next);
      void saveDictationLanguage(next);
    },
  };
}
const AUTOMATIC = "automatic";

/** "Current language: English. Tap to change", opening the language menu. */
function LanguageLine({
  language,
}: {
  language: ReturnType<typeof useDictationLanguage>;
}) {
  const { colors } = useTheme();
  const menu = usePopupMenu({
    options: language.options,
    value: language.value,
    onChange: language.choose,
  });
  return (
    <View ref={menu.anchor} collapsable={false}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Speech language, ${language.label}`}
        accessibilityHint="Opens a menu to choose the dictation language"
        hitSlop={8}
        onPress={menu.open}
      >
        <Text
          style={[styles.languageLine, { color: colors.textSecondary }]}
        >
          Current language:{" "}
          <Text style={{ color: colors.primary }}>{language.label}</Text>.
          Tap to change
        </Text>
      </Pressable>
      {menu.popup}
    </View>
  );
}

/**
 * One-time explanation as a floating card over the chat (the mockup's
 * "Speech to Text" card): what it does, microphone access, language, Start.
 */
function DictationIntro({
  visible,
  language,
  onClose,
  onStart,
}: {
  visible: boolean;
  language: ReturnType<typeof useDictationLanguage>;
  onClose: () => void;
  onStart: () => void;
}) {
  const { colors, mode } = useTheme();
  const insets = useSafeAreaInsets();
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
        style={[styles.introFloat, { marginBottom: Math.max(insets.bottom, 12) + 4 }]}
      >
        <LinearGradient
          colors={mode === "dark" ? ["#26242C", "#1D1B22"] : ["#FFFFFF", "#F2EFF5"]}
          style={styles.intro}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Close"
            onPress={onClose}
            hitSlop={8}
            style={[styles.introClose, { backgroundColor: colors.card }]}
          >
            <X size={18} color={colors.textPrimary} />
          </Pressable>
          <View style={styles.introHead}>
            <Mic size={34} color={colors.textPrimary} />
            <Text
              accessibilityRole="header"
              style={[styles.introTitle, { color: colors.textPrimary }]}
            >
              Speech to Text
            </Text>
            <Text style={[styles.introLead, { color: colors.textSecondary }]}>
              Speak instead of typing to write faster. You review the text
              before it goes into your message.
            </Text>
          </View>
          <View style={[styles.introCard, { backgroundColor: colors.card }]}>
            <View style={styles.introRow}>
              <Mic size={18} color={colors.textMuted} />
              <View style={styles.introRowText}>
                <Text
                  style={[styles.introRowTitle, { color: colors.textPrimary }]}
                >
                  Access permission
                </Text>
                <Text
                  style={[styles.introRowDetail, { color: colors.textSecondary }]}
                >
                  Allow microphone and speech recognition access to turn your
                  voice into text. Speech is recognized on your iPhone when
                  available.
                </Text>
              </View>
            </View>
            <View
              style={[
                styles.introRow,
                {
                  borderTopWidth: StyleSheet.hairlineWidth,
                  borderTopColor: colors.borderSoft,
                },
              ]}
            >
              <Globe size={18} color={colors.textMuted} />
              <View style={styles.introRowText}>
                <Text
                  style={[styles.introRowTitle, { color: colors.textPrimary }]}
                >
                  Speech language
                </Text>
                <LanguageLine language={language} />
              </View>
            </View>
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
              Let's Start
            </Text>
          </Pressable>
        </LinearGradient>
      </View>
    </Modal>
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
  introBackdrop: { flex: 1 },
  // A floating card inset from the screen edges, as in the mockup.
  introFloat: {
    marginHorizontal: 12,
    borderRadius: 34,
    shadowColor: "#1A1530",
    shadowOpacity: 0.18,
    shadowRadius: 24,
    shadowOffset: { width: 0, height: 10 },
  },
  intro: {
    borderRadius: 34,
    paddingHorizontal: 20,
    paddingTop: 16,
    paddingBottom: 20,
    gap: 20,
    overflow: "hidden",
  },
  introRowText: { flex: 1, gap: 3 },
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
  languageLine: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
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
