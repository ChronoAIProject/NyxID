import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { AppState, ScrollView, StyleSheet, View, Text } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import * as Speech from "expo-speech";
import { Mic, MicOff, PhoneOff } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useWorkspaceAccessibility } from "../../lib/accessibility/workspaceAccessibility";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useSharedValue } from "react-native-reanimated";
import {
  VoiceComposerShell,
  VoiceComposerHeader,
  VoiceComposerTextRow,
  VoiceComposerControls,
  VoiceMainButton,
  VoiceIconButton,
  useVoiceComposerHeight,
} from "../assistant/voice/VoiceComposerShell";
import { voiceSpeechText } from "../assistant/voice/voiceSpeechText";
import { useComposerDictation } from "../assistant/useComposerDictation";
import type { GroupMessage } from "./groupContracts";

type Props = {
  scope: string;
  crewName: string;
  enabled: boolean;
  busy?: boolean;
  messages: readonly GroupMessage[];
  onSend: (text: string) => Promise<boolean>;
  onClose: () => void;
  minimized: boolean;
  onExpand: () => void;
  draft: string;
  onChangeText: (text: string) => void;
  canSendText: boolean;
  onSendText: () => void;
  recipientLabel: string;
  captureKey: string;
  captureControl?: { current: (() => void) | undefined };
};

const SILENCE_REARM_MS = 500;
const RAPID_SILENCE_MS = 1000;
const MAX_RAPID_SILENCES = 3;
const MICROPHONE_RETRY = "Can't hear the microphone. Tap to retry.";

/** Native speech uses the ordinary crew sender and its recipient/authority fences. */
export function CrewVoiceComposer(props: Props) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const reducedMotion = useReducedMotion();
  const panelHeight = useVoiceComposerHeight();
  const inputLevel = useSharedValue(0);
  const outputLevel = useSharedValue(0);
  const [foreground, setForeground] = useState(true);
  const appActive = useRef(true);
  const closed = useRef(false);
  const { sheetOpen } = useWorkspaceAccessibility();
  const epoch = getAuthEpoch();
  const origin = useRef({ scope: props.scope, epoch });
  const [paused, setPaused] = useState(false);
  const [rearming, setRearming] = useState(false);
  const rearmTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const rapidSilences = useRef(0);
  const [sending, setSending] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const [error, setError] = useState<string>();
  const [connectedAt, setConnectedAt] = useState<number>();
  const [now, setNow] = useState(Date.now());
  const mounted = useRef(true);
  const current = useRef(props);
  current.current = props;
  const watermark = useRef(
    Math.max(0, ...props.messages.map((row) => row.seq)),
  );
  const queue = useRef<string[]>([]);
  const playback = useRef(0);
  const playing = useRef(false);
  const sendLock = useRef(false);
  const captureKey = props.captureKey;
  const sendUtterance = props.onSend;
  const available = () =>
    mounted.current &&
    !closed.current &&
    appActive.current &&
    origin.current.epoch === getAuthEpoch() &&
    origin.current.scope === current.current.scope &&
    current.current.enabled;
  function clearRearm() {
    clearTimeout(rearmTimer.current);
    rearmTimer.current = undefined;
  }
  const dictation = useComposerDictation({
    scope: `crew-voice:${props.scope}:${props.captureKey}`,
    enabled:
      props.enabled &&
      focused &&
      foreground &&
      !sheetOpen &&
      !props.busy &&
      !paused &&
      !sending &&
      !speaking &&
      !error,
    continuous: false,
    onSilence: (durationMs) => {
      if (!available()) return;
      clearRearm();
      rapidSilences.current =
        durationMs < RAPID_SILENCE_MS ? rapidSilences.current + 1 : 0;
      if (rapidSilences.current >= MAX_RAPID_SILENCES) {
        setError(MICROPHONE_RETRY);
        return;
      }
      setRearming(true);
      rearmTimer.current = setTimeout(() => {
        rearmTimer.current = undefined;
        if (available()) setRearming(false);
      }, SILENCE_REARM_MS);
    },
    onInsert: async (text) => {
      if (
        !available() ||
        current.current.busy ||
        sendLock.current ||
        current.current.captureKey !== captureKey
      )
        return;
      rapidSilences.current = 0;
      sendLock.current = true;
      setSending(true);
      try {
        const delivered = await sendUtterance(text);
        if (available() && !delivered)
          setError(
            "Your spoken message was not sent. Review the draft in chat before trying again.",
          );
      } catch {
        if (available())
          setError(
            "Your spoken message could not be sent. Review the crew before trying again.",
          );
      } finally {
        sendLock.current = false;
        if (mounted.current && epoch === getAuthEpoch()) setSending(false);
      }
    },
  });
  const recognition = useRef(dictation);
  recognition.current = dictation;
  useLayoutEffect(() => {
    const control = props.captureControl;
    if (!control) return;
    control.current = () => recognition.current.discard();
    return () => { control.current = undefined; };
  }, [props.captureControl]);

  function stopPlayback() {
    playback.current++;
    playing.current = false;
    queue.current = [];
    void Speech.stop().catch(() => {});
  }
  function close() {
    closed.current = true;
    clearRearm();
    setForeground(false);
    recognition.current.discard();
    stopPlayback();
    props.onClose();
  }
  useEffect(() => {
    mounted.current = true;
    const subscription = AppState.addEventListener("change", (state) => {
      if (state === "active") {
        appActive.current = true;
        if (!closed.current) setForeground(true);
      } else if (
        !(state === "inactive" && recognition.current.phase === "permission")
      ) {
        appActive.current = false;
        clearRearm();
        setRearming(false);
        setForeground(false);
        recognition.current.discard();
        stopPlayback();
        setSpeaking(false);
        if (state === "background") {
          closed.current = true;
          current.current.onClose();
        }
      }
    });
    return () => {
      mounted.current = false;
      clearRearm();
      subscription.remove();
      stopPlayback();
    };
  }, []);
  useEffect(() => {
    if (origin.current.scope !== props.scope || origin.current.epoch !== epoch)
      close();
    return clearRearm;
  }, [props.scope, epoch]);
  useEffect(() => {
    if (!focused) close();
    if (
      !props.enabled ||
      paused ||
      sheetOpen ||
      !foreground ||
      error ||
      dictation.error
    ) {
      stopPlayback();
      setSpeaking(false);
    }
  }, [
    focused,
    props.enabled,
    paused,
    sheetOpen,
    foreground,
    error,
    dictation.error,
  ]);
  useEffect(() => {
    if (
      !props.enabled ||
      !focused ||
      !foreground ||
      sheetOpen ||
      props.busy ||
      paused ||
      sending ||
      speaking ||
      error ||
      dictation.error
    ) {
      clearRearm();
      setRearming(false);
      rapidSilences.current = 0;
    }
  }, [
    props.enabled,
    focused,
    foreground,
    sheetOpen,
    props.busy,
    paused,
    sending,
    speaking,
    error,
    dictation.error,
  ]);
  useEffect(() => {
    if (dictation.recording)
      setConnectedAt((previous) => previous ?? Date.now());
    if (
      available() &&
      focused &&
      foreground &&
      !sheetOpen &&
      !props.busy &&
      !paused &&
      !sending &&
      !speaking &&
      !error &&
      !dictation.error &&
      !rearming &&
      !dictation.active
    )
      void dictation.start();
  }, [
    props.enabled,
    focused,
    foreground,
    sheetOpen,
    props.busy,
    paused,
    sending,
    speaking,
    error,
    dictation.active,
    dictation.error,
    dictation.recording,
    rearming,
    props.scope,
    props.captureKey,
    epoch,
  ]);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);

  useEffect(() => {
    const fresh = props.messages
      .filter((row) => row.seq > watermark.current)
      .sort((a, b) => a.seq - b.seq);
    watermark.current = Math.max(
      watermark.current,
      ...fresh.map((row) => row.seq),
    );
    if (
      !available() ||
      paused ||
      error ||
      dictation.error ||
      !focused ||
      sheetOpen
    )
      return;
    queue.current.push(
      ...fresh
        .filter((row) => row.role === "agent" && row.text.trim())
        .map((row) => voiceSpeechText(row.text))
        .filter(Boolean),
    );
    if (playing.current || !queue.current.length) return;
    recognition.current.discard();
    setSpeaking(true);
    playing.current = true;
    const ticket = ++playback.current;
    const next = () => {
      if (!available() || ticket !== playback.current) return;
      const text = queue.current.shift();
      if (!text) {
        playing.current = false;
        setSpeaking(false);
        return;
      }
      let finished = false;
      const finish = () => {
        if (!finished) {
          finished = true;
          next();
        }
      };
      try {
        Speech.speak(text, {
          useApplicationAudioSession: false,
          onDone: finish,
          onStopped: finish,
          onError: () => {
            if (!available() || ticket !== playback.current || finished) return;
            setError(
              "The reply could not be read aloud. It is available in the transcript.",
            );
            finished = true;
            stopPlayback();
            setSpeaking(false);
          },
        });
      } catch {
        setError("Speech playback is unavailable. Continue in the transcript.");
        stopPlayback();
        setSpeaking(false);
      }
    };
    next();
  }, [
    props.messages,
    props.enabled,
    paused,
    focused,
    sheetOpen,
    foreground,
    error,
    dictation.error,
  ]);

  const notice = error ?? dictation.error;
  const status =
    !props.enabled || paused || sheetOpen || !foreground
      ? "Microphone paused"
      : notice
        ? "Voice paused"
        : speaking
          ? "Reading crew reply…"
          : sending || props.busy
            ? "Sending to crew…"
            : dictation.recording
              ? "Listening…"
              : "Connecting…";
  const seconds =
    connectedAt === undefined
      ? undefined
      : Math.max(0, Math.floor((now - connectedAt) / 1000));
  const duration =
    seconds === undefined
      ? "-"
      : `${String(Math.floor(seconds / 60)).padStart(2, "0")}:${String(seconds % 60).padStart(2, "0")}`;
  useEffect(
    () =>
      dictation.subscribeLevel((level) => {
        inputLevel.value = level;
      }),
    [dictation.subscribeLevel, inputLevel],
  );
  const togglePause = () => {
    clearRearm();
    setRearming(false);
    rapidSilences.current = 0;
    recognition.current.discard();
    setError(undefined);
    setPaused((value) => !value);
  };
  return (
    <VoiceComposerShell
      minimized={props.minimized}
      reducedMotion={reducedMotion}
      phase={
        notice || paused || !foreground || !props.enabled
          ? "muted"
          : speaking
            ? "speaking"
            : sending || props.busy
              ? "thinking"
              : dictation.recording
                ? "listening"
                : "connecting"
      }
      input={inputLevel}
      output={outputLevel}
    >
      {!props.minimized && (
        <ScrollView
          style={{ maxHeight: Math.max(44, panelHeight - 64) }}
          keyboardShouldPersistTaps="handled"
          contentContainerStyle={styles.content}
        >
          <VoiceComposerHeader
            name={props.crewName}
            status={status}
            duration={duration}
            error={!!notice}
          />
          <Label muted>
            To {props.recipientLabel}. Speak to send. New agent replies are read
            aloud.
          </Label>
          {dictation.text ? <Label>{dictation.text}</Label> : null}
          {notice ? (
            <Label accessibilityRole="alert" style={{ color: colors.danger }}>
              {notice}
            </Label>
          ) : null}
          <VoiceComposerControls>
            <View style={styles.sideSpacer} />
            <View style={styles.holdColumn}>
              <VoiceMainButton
                label={
                  notice
                    ? "Retry crew voice"
                    : paused
                      ? "Resume crew microphone"
                      : "Pause crew microphone"
                }
                fill={paused || notice ? colors.danger : colors.textPrimary}
                disabled={!props.enabled || !foreground}
                onPress={
                  notice
                    ? () => {
                        clearRearm();
                        setRearming(false);
                        rapidSilences.current = 0;
                        dictation.discard();
                        setError(undefined);
                        setPaused(false);
                      }
                    : togglePause
                }
              >
                {paused ? (
                  <MicOff color={colors.onPrimary} size={26} />
                ) : (
                  <Mic
                    color={paused || notice ? colors.onPrimary : colors.bg}
                    size={26}
                  />
                )}
              </VoiceMainButton>
              <Text style={[styles.holdHint, { color: colors.textSecondary }]}>
                {paused ? "Paused" : "Hands-free"}
              </Text>
            </View>
            <VoiceIconButton large label="End voice and close" onPress={close}>
              <PhoneOff color={colors.danger} size={20} />
            </VoiceIconButton>
          </VoiceComposerControls>
        </ScrollView>
      )}
      <VoiceComposerTextRow
        minimized={props.minimized}
        status={status}
        duration={duration}
        inputLabel="Message during crew voice"
        draft={props.draft}
        onExpand={props.onExpand}
        onChangeText={(text) => {
          clearRearm();
          setRearming(false);
          recognition.current.discard();
          stopPlayback();
          setSpeaking(false);
          props.onChangeText(text);
        }}
        onSendText={props.onSendText}
        canSendText={props.canSendText}
        onClose={close}
      />
    </VoiceComposerShell>
  );
}
const styles = StyleSheet.create({
  content: { gap: 10 },
  sideSpacer: { width: 48, height: 48 },
  holdColumn: { alignItems: "center", gap: 6 },
  holdHint: {
    fontFamily: fonts.bodySemi,
    fontSize: 12,
    lineHeight: 16,
    minHeight: 16,
  },
});
