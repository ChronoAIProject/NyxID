import { useEffect, useRef, useState } from "react";
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  TextInput,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import * as Speech from "expo-speech";
import { Mic, MicOff, PhoneOff } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useWorkspaceAccessibility } from "../../lib/accessibility/workspaceAccessibility";
import { getAuthEpoch } from "../../lib/auth/authEpoch";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  ChatComposerFrame,
  ComposerSubmitButton,
} from "../assistant/ChatComposer";
import { useComposerDictation } from "../assistant/useComposerDictation";
import type { GroupMessage } from "./groupContracts";

type Props = {
  scope: string;
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
};

/** Native speech uses the ordinary crew sender and its recipient/authority fences. */
export function CrewVoiceComposer(props: Props) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const { sheetOpen } = useWorkspaceAccessibility();
  const epoch = getAuthEpoch();
  const [paused, setPaused] = useState(false);
  const [sending, setSending] = useState(false);
  const [speaking, setSpeaking] = useState(false);
  const [error, setError] = useState<string>();
  const [connectedAt, setConnectedAt] = useState<number>();
  const [now, setNow] = useState(Date.now());
  const mounted = useRef(true);
  const current = useRef(props);
  current.current = props;
  const watermark = useRef(Math.max(0, ...props.messages.map((row) => row.seq)));
  const queue = useRef<string[]>([]);
  const playback = useRef(0);
  const playing = useRef(false);
  const sendLock = useRef(false);
  const available = () =>
    mounted.current && epoch === getAuthEpoch() && current.current.enabled;
  const dictation = useComposerDictation({
    scope: `crew-voice:${props.scope}`,
    enabled:
      props.enabled &&
      focused &&
      !sheetOpen &&
      !props.busy &&
      !paused &&
      !sending &&
      !speaking &&
      !error,
    continuous: false,
    onInsert: async (text) => {
      if (!available() || current.current.busy || sendLock.current) return;
      sendLock.current = true;
      setSending(true);
      try {
        const delivered = await current.current.onSend(text);
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

  function stopPlayback() {
    playback.current++;
    playing.current = false;
    queue.current = [];
    void Speech.stop().catch(() => {});
  }
  function close() {
    recognition.current.discard();
    stopPlayback();
    props.onClose();
  }
  useEffect(() => {
    mounted.current = true;
    const subscription = AppState.addEventListener("change", (state) => {
      // iOS permission prompts temporarily make the app inactive. The speech
      // hook waits for foreground after those prompts before opening capture.
      if (
        state !== "active" &&
        !(state === "inactive" && recognition.current.phase === "permission")
      ) {
        recognition.current.discard();
        stopPlayback();
        setSpeaking(false);
        setPaused(true);
        if (state === "background") current.current.onClose();
      }
    });
    return () => {
      mounted.current = false;
      subscription.remove();
      stopPlayback();
    };
  }, []);
  useEffect(() => {
    if (!focused) close();
    if (!props.enabled || paused || sheetOpen) {
      stopPlayback();
      setSpeaking(false);
    }
  }, [focused, props.enabled, paused, sheetOpen]);
  useEffect(() => {
    if (dictation.recording)
      setConnectedAt((previous) => previous ?? Date.now());
    if (
      props.enabled &&
      focused &&
      !sheetOpen &&
      !props.busy &&
      !paused &&
      !sending &&
      !speaking &&
      !error &&
      !dictation.error &&
      !dictation.active
    )
      void dictation.start();
  }, [
    props.enabled,
    focused,
    sheetOpen,
    props.busy,
    paused,
    sending,
    speaking,
    error,
    dictation.active,
    dictation.error,
    dictation.recording,
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
    if (!available() || paused || !focused || sheetOpen) return;
    queue.current.push(
      ...fresh
        .filter((row) => row.role === "agent" && row.text.trim())
        .map((row) => row.text),
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
            if (available() && ticket === playback.current)
              setError(
                "The reply could not be read aloud. It is available in the transcript.",
              );
            finish();
          },
        });
      } catch {
        setError("Speech playback is unavailable. Continue in the transcript.");
        finish();
      }
    };
    next();
  }, [props.messages, props.enabled, paused, focused, sheetOpen]);

  const notice = error ?? dictation.error;
  const status =
    !props.enabled || paused || sheetOpen
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
  return (
    <ChatComposerFrame bounce={false}>
      {!props.minimized && (
        <ScrollView style={styles.body} contentContainerStyle={styles.content}>
          <Label>
            Crew voice · {status} · {duration}
          </Label>
          <Label muted>
            To {props.recipientLabel}. Speak to send. New agent replies are read
            aloud.
          </Label>
          {dictation.text ? <Label>{dictation.text}</Label> : null}
          {notice ? <Label accessibilityRole="alert">{notice}</Label> : null}
          {notice ? (
            <ActionButton
              label="Retry crew voice"
              disabled={!props.enabled}
              onPress={() => {
                dictation.discard();
                setError(undefined);
              }}
            />
          ) : null}
          <ActionButton
            label={paused ? "Resume crew microphone" : "Pause crew microphone"}
            variant="secondary"
            onPress={() => setPaused((value) => !value)}
          />
        </ScrollView>
      )}
      <View style={styles.row}>
        <Pressable
          style={styles.control}
          accessibilityRole="button"
          accessibilityLabel="Expand crew voice"
          onPress={props.onExpand}
        >
          {paused ? (
            <MicOff color={colors.textPrimary} size={20} />
          ) : (
            <Mic color={colors.textPrimary} size={20} />
          )}
        </Pressable>
        <TextInput
          style={[styles.input, { color: colors.textPrimary }]}
          accessibilityLabel="Message during crew voice"
          value={props.draft}
          placeholder={status}
          placeholderTextColor={colors.textSecondary}
          onFocus={props.onExpand}
          onChangeText={(text) => {
            recognition.current.discard();
            stopPlayback();
            setSpeaking(false);
            props.onExpand();
            props.onChangeText(text);
          }}
          returnKeyType="send"
          onSubmitEditing={() => {
            if (props.canSendText) props.onSendText();
          }}
        />
        {props.minimized ? <Label muted>{duration}</Label> : null}
        {props.draft.trim() ? (
          <ComposerSubmitButton
            kind="send"
            label="Send message during crew voice"
            disabled={!props.canSendText}
            onPress={props.onSendText}
          />
        ) : null}
        <Pressable
          style={styles.control}
          accessibilityRole="button"
          accessibilityLabel="End crew voice"
          onPress={close}
        >
          <PhoneOff color={colors.danger} size={20} />
        </Pressable>
      </View>
    </ChatComposerFrame>
  );
}

const styles = StyleSheet.create({
  body: { maxHeight: 180, flexGrow: 0 },
  content: { gap: 10, padding: 12 },
  row: { flexDirection: "row", alignItems: "center", minHeight: 44 },
  control: {
    width: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  input: {
    flex: 1,
    minWidth: 0,
    minHeight: 44,
    fontFamily: fonts.body,
    fontSize: 14,
  },
});
