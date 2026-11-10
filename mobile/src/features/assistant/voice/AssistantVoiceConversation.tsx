import { useReducedMotion } from "../../../lib/accessibility/useReducedMotion";
import { useEffect, useRef, useState } from "react";
import {
  ActivityIndicator,
  Animated,
  LayoutAnimation,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
  useWindowDimensions,
} from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import {
  CircleHelp,
  Mic,
  MicOff,
  Phone,
  Radio,
  PhoneOff,
  Volume2,
  VolumeX,
  X,
} from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../../components/workspace/WorkspaceUI";
import { SettingsRow } from "../../../components/settings/SettingsList";
import { WorkspaceSelectionSheet } from "../../../components/workspace/WorkspaceSelectionField";
import {
  ScrollEdgeFades,
  useScrollEdges,
} from "../../../components/workspace/ScrollEdgeFades";
import { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { useNetworkStatus } from "../../../hooks/useNetworkStatus";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { DictationWaveform } from "../DictationWaveform";
import { ChatComposerFrame } from "../ChatComposer";
import { useNativeVoice } from "./useNativeVoice";
import { useVoiceLevels } from "./useVoiceLevels";
import {
  VoiceComposerShell,
  VoiceComposerHeader,
  VoiceComposerCompactRow,
  VoiceComposerControls,
  useVoiceComposerHeight,
  VoiceIconButton as IconButton,
  VoiceMainButton as MainButton,
} from "./VoiceComposerShell";
import { VoiceMediaBridge } from "./VoiceMediaBridge";
import { voiceAuraPhase, voiceAuraStatus } from "./voiceAuraState";
import {
  chooseVoiceOption,
  voiceElapsed,
  voiceOptionKey,
} from "./voiceSelection";
import type {
  VoiceOption,
  VoicePreferences,
  VoiceSnapshot,
} from "./voiceContracts";

type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  minimized?: boolean;
  onExpand?: () => void;
  onUpdate: () => void;
  /** The main chat shows the call's finished words; this composer shows unsent ones. */
  onSnapshot?: (snapshot: VoiceSnapshot | undefined) => void;
  /** Controlled by the chat's Chat options › Voice preferences action. */
  preferencesOpen?: boolean;
  onPreferencesClose?: () => void;
};
type CallSelection = { option: VoiceOption; preferences: VoicePreferences };

/**
 * Voice mode is a composer: it slides up in place of the message composer,
 * so the chat above, with its cards and links, stays on screen and usable
 * during the call.
 */
export function AssistantVoiceConversation(props: Props) {
  const [setup] = useState(() => {
    try {
      return { environment: nativeVoiceEnvironment() };
    } catch {
      return {
        error:
          "Voice needs the configured NyxID frontend and API addresses. You can continue chatting with text.",
      };
    }
  });
  return setup.environment ? (
    <VoiceConversation
      key={`${props.thread}:${setup.environment.epoch}`}
      {...props}
      environment={setup.environment}
    />
  ) : (
    <ChatComposerFrame style={styles.frame} bounce={false}>
      <Notice message={setup.error ?? "Voice is unavailable."} tone="error" />
      <ActionButton
        label="Return to chat"
        variant="secondary"
        onPress={props.onClose}
      />
    </ChatComposerFrame>
  );
}

function VoiceConversation({
  thread,
  agentName,
  onClose,
  onUpdate,
  minimized = false,
  onExpand,
  onSnapshot,
  preferencesOpen = false,
  onPreferencesClose,
  environment,
}: Props & {
  environment: ReturnType<typeof nativeVoiceEnvironment>;
}) {
  const { colors } = useTheme();
  const panelHeight = useVoiceComposerHeight();
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const levels = useVoiceLevels(voice.media);
  const [hold] = useState(() => new Animated.Value(0));
  const [rise] = useState(() => new Animated.Value(reducedMotion ? 0 : 1));
  const transcriptEdges = useScrollEdges();
  const [helpOpen, setHelpOpen] = useState(false);
  const helpAnchor = useRef<View>(null);
  const [anchor, setAnchor] = useState({ x: 0, y: 0, height: 44 });
  const [helpHeight, setHelpHeight] = useState(240);
  const { width, height, fontScale } = useWindowDimensions();
  const insets = useSafeAreaInsets();
  const helpWidth = Math.min(320, width - insets.left - insets.right - 32);
  const helpTop = Math.max(
    insets.top + 16,
    anchor.y + anchor.height + helpHeight + 16 < height - insets.bottom
      ? anchor.y + anchor.height + 8
      : anchor.y - helpHeight - 8,
  );
  function openHelp() {
    helpAnchor.current?.measureInWindow((x, y, _width, height) =>
      setAnchor({ x, y, height }),
    );
    setHelpOpen(true);
  }
  const [now, setNow] = useState(Date.now());
  const [voicePickerScope, setVoicePickerScope] = useState<string>();
  const [showErrorDetails, setShowErrorDetails] = useState(false);
  const [billingOpenError, setBillingOpenError] = useState(false);
  const [preferencesError, setPreferencesError] = useState(false);
  const [savingPreferences, setSavingPreferences] = useState(false);
  const [modeChoice, setModeChoice] =
    useState<VoicePreferences["input_mode"]>();
  const [voiceChoice, setVoiceChoice] = useState<string>();
  const [callSelection, setCallSelection] = useState<CallSelection>();
  const [endRequested, setEndRequested] = useState(false);
  const [endingLocal, setEndingLocal] = useState(false);
  const [checkingLocal, setCheckingLocal] = useState(false);
  const transcript = useRef<ScrollView>(null);
  const entry = useRef({ thread, epoch: environment.epoch, attempted: false });
  const closed = useRef(false);
  const mounted = useRef(true);
  const endBusy = useRef(false);
  const checkBusy = useRef(false);
  const saveBusy = useRef(false);
  const callbacks = useRef({ onSnapshot });
  callbacks.current = { onSnapshot };
  if (
    entry.current.thread !== thread ||
    entry.current.epoch !== environment.epoch
  ) {
    entry.current = { thread, epoch: environment.epoch, attempted: false };
  }
  const options = useQuery({
    queryKey: ["workspace", "voice-options", environment.epoch, thread],
    queryFn: ({ signal }) => environment.api.options(thread, signal),
    enabled: focused && isConnected,
    retry: false,
  });
  const settings = useQuery({
    queryKey: ["workspace", "voice-settings", environment.epoch],
    queryFn: ({ signal }) => environment.api.settings(signal),
    enabled: focused && isConnected,
    retry: false,
  });
  const saved = settings.data?.voice;
  const choices = options.data?.options ?? [];
  const selectionReady = options.isSuccess && settings.isSuccess;
  const option = selectionReady ? chooseVoiceOption(choices, saved) : undefined;
  const isGrok = option?.voice.protocol === "xai_realtime";
  const nextMode = isGrok
    ? "push_to_talk"
    : (modeChoice ?? saved?.input_mode ?? "automatic");
  const nextVoice =
    voiceChoice ?? saved?.voice ?? option?.voice.voices[0]?.id ?? null;
  const missingVoice =
    !!option &&
    !!nextVoice &&
    !option.voice.voices.some((row) => row.id === nextVoice);
  // A live call keeps the source and voice it started with; the header toggle
  // can still switch its listening mode.
  const inputMode = callSelection?.preferences.input_mode ?? nextMode;
  const currentOption = callSelection?.option ?? option;
  const currentVoice = callSelection?.preferences.voice ?? nextVoice;
  const voiceLabel = currentOption?.voice.voices.find(
    (row) => row.id === currentVoice,
  )?.label;
  const session = voice.snapshot?.session;
  const sessionTerminal =
    session?.state === "closed" || session?.state === "failed";
  const conflictTerminal =
    voice.conflict?.state === "closed" || voice.conflict?.state === "failed";
  // A terminal session may retain end_requested; it has already finished.
  // Its final snapshot also supersedes the same call's pending cleanup metadata.
  const conflict =
    voice.conflict &&
    !conflictTerminal &&
    !(sessionTerminal && voice.conflict.id === session?.id)
      ? voice.conflict
      : null;
  const confirmedEnd =
    voice.confirmedEnded &&
    !voice.starting &&
    !voice.connected &&
    !conflict &&
    !voice.conflictUnknown;
  const terminal =
    sessionTerminal || (!session && conflictTerminal) || confirmedEnd;
  const active = voice.starting || voice.connected;
  const uncertain = voice.conflictUnknown;
  const confirmingEnd =
    (endRequested && !terminal) ||
    !!conflict?.end_requested ||
    conflict?.state === "closing" ||
    (!terminal && (session?.state === "closing" || !!session?.end_requested));
  const checking = checkingLocal || voice.checkingConflict;
  const ending = endingLocal || voice.endingConflict;
  const muted = session?.muted ?? false;
  const microphoneBlocked =
    muted ||
    !!session?.input_muted ||
    confirmingEnd ||
    session?.state !== "active";
  const outputActive =
    voice.connected && levels.speaking && !voice.speakerMuted && !confirmingEnd;
  const allCaptions = voice.snapshot?.captions ?? [];
  // Only words not yet sent show here; the chat holds everything already sent.
  const captions = allCaptions.filter(
    (caption) =>
      caption.speaker === "user" && !caption.sealed && !!caption.text.trim(),
  );
  const tasks = voice.snapshot?.tasks ?? [];
  // Listening follows the client's actual input gate and Speaking follows
  // playback; a stale partial caption never invents audio activity.
  const phase = voiceAuraPhase({
    starting: voice.starting,
    connected: voice.connected,
    inputMode,
    holding: voice.holding,
    microphoneBlocked: microphoneBlocked || !voice.inputEnabled,
    muted,
    outputActive,
    captions: allCaptions.map((caption) =>
      caption.speaker === "assistant"
        ? { ...caption, complete: true }
        : caption,
    ),
    tasks,
  });
  const hasCall =
    active ||
    (!!session && !terminal) ||
    !!conflict ||
    uncertain ||
    confirmingEnd;
  const canStart =
    !!option?.available &&
    !missingVoice &&
    !hasCall &&
    !conflict &&
    !uncertain &&
    !checking &&
    !ending &&
    !confirmingEnd &&
    isConnected &&
    focused &&
    selectionReady;
  const canChangePreferences =
    selectionReady && !!option?.available && isConnected && !savingPreferences;
  const selectionScope = JSON.stringify([
    environment.epoch,
    thread,
    option ? voiceOptionKey(option) : null,
  ]);
  const pickerContext = useRef({ scope: selectionScope, allowed: false });
  pickerContext.current = {
    scope: selectionScope,
    allowed:
      preferencesOpen &&
      !minimized &&
      focused &&
      canChangePreferences &&
      voicePickerScope === selectionScope,
  };
  const status = checking
    ? "Checking call status…"
    : ending
      ? "Ending…"
      : confirmingEnd
        ? "Finishing call…"
        : uncertain
          ? "Call status unconfirmed"
          : conflict
            ? "Call already active"
            : !isConnected
              ? "Offline"
              : voice.starting
                ? "Connecting…"
                : voice.connected
                  ? voiceAuraStatus(phase, inputMode)
                  : voice.error || voice.recovery
                    ? "Unable to start"
                    : terminal
                      ? "Call ended"
                      : missingVoice
                        ? "Choose a voice"
                        : options.error ||
                            settings.error ||
                            (selectionReady && !option?.available)
                          ? "Voice unavailable"
                          : "Connecting…";
  // "–" until the provider is ready, then the call clock from 00:00.
  const duration =
    voice.starting || voice.connectedAt === undefined
      ? "-"
      : voiceElapsed(
          new Date(voice.connectedAt).toISOString(),
          null,
          voice.disconnectedAt ?? now,
        ).padStart(5, "0");
  const needsRetry =
    (entry.current.attempted ||
      !!voice.error ||
      !!voice.recovery ||
      !!voice.snapshot) &&
    !hasCall;

  useEffect(() => {
    mounted.current = true;
    closed.current = false;
    return () => {
      mounted.current = false;
      callbacks.current.onSnapshot?.(undefined);
    };
  }, [thread, environment]);
  useEffect(() => {
    callbacks.current.onSnapshot?.(voice.snapshot ?? undefined);
  }, [voice.snapshot]);
  useEffect(() => {
    if (!active && !confirmingEnd) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active, confirmingEnd]);
  useEffect(() => {
    setShowErrorDetails(false);
  }, [voice.error]);
  useEffect(() => {
    // A mounted live call also keeps its original source and voice while the
    // user edits next-call preferences or the settings query refreshes.
    if (active && !callSelection && option) {
      const chosen = preferences(nextMode);
      if (chosen) setCallSelection({ option, preferences: chosen });
    }
  }, [active, callSelection, option, nextMode]);
  useEffect(() => {
    // One explicit entry owns one automatic attempt. A conflict, failed start
    // or existing call must never silently turn into a replacement call.
    if (
      active ||
      voice.snapshot ||
      voice.conflict ||
      uncertain ||
      voice.error ||
      voice.recovery
    )
      entry.current.attempted = true;
    if (!entry.current.attempted && canStart) {
      entry.current.attempted = true;
      void start();
    }
  }, [
    selectionReady,
    canStart,
    active,
    voice.snapshot,
    conflict,
    uncertain,
    voice.error,
    voice.recovery,
  ]);
  useEffect(() => {
    if (!isConnected && active && !conflict && !uncertain) void requestEnd();
  }, [isConnected]);
  useEffect(() => {
    if (!focused && active && !conflict && !uncertain) void requestEnd();
  }, [focused]);
  useEffect(() => {
    if (
      (endRequested || confirmedEnd) &&
      !ending &&
      !checking &&
      !conflict &&
      !uncertain &&
      !active &&
      terminal
    )
      closeOnce();
  }, [
    endRequested,
    confirmedEnd,
    ending,
    checking,
    conflict,
    uncertain,
    active,
    terminal,
  ]);
  useEffect(() => {
    if (voice.holding) voice.hold(false);
    setHelpOpen(false);
  }, [minimized]);
  useEffect(() => {
    if (!preferencesOpen || !focused || minimized)
      setVoicePickerScope(undefined);
  }, [preferencesOpen, focused, minimized]);

  // Holding the microphone swells the button and lifts a soft halo around it;
  // releasing settles it back. Reduce Motion switches without animating.
  useEffect(() => {
    if (reducedMotion) {
      hold.setValue(voice.holding ? 1 : 0);
      return;
    }
    const animation = voice.holding
      ? Animated.spring(hold, {
          toValue: 1,
          speed: 16,
          bounciness: 9,
          useNativeDriver: true,
        })
      : Animated.timing(hold, {
          toValue: 0,
          duration: 220,
          useNativeDriver: true,
        });
    animation.start();
    return () => animation.stop();
  }, [voice.holding, reducedMotion]);

  // The voice composer slides up from where the message composer was.
  useEffect(() => {
    if (reducedMotion) return;
    const animation = Animated.spring(rise, {
      toValue: 0,
      speed: 14,
      bounciness: 4,
      useNativeDriver: true,
    });
    animation.start();
    return () => animation.stop();
  }, []);

  function closeOnce() {
    if (!mounted.current || closed.current) return;
    closed.current = true;
    onClose();
  }
  function returnToText() {
    // An explicit End keeps this media owner alive until its bounded drain
    // finishes. Status reads may be left without ending a different call.
    if (endBusy.current || voice.endingConflict) return;
    closeOnce();
  }
  function preferences(
    mode: VoicePreferences["input_mode"],
    voiceId = nextVoice,
  ): VoicePreferences | undefined {
    if (
      !selectionReady ||
      !option?.available ||
      (voiceId && !option.voice.voices.some((row) => row.id === voiceId))
    )
      return;
    return {
      service_id: option.service_id,
      connection_id: option.connection_id,
      key_source: option.key_source,
      model: option.model,
      voice: voiceId,
      input_mode: isGrok ? "push_to_talk" : mode,
      language: saved?.language ?? null,
      notify_on_completion: saved?.notify_on_completion ?? false,
    };
  }
  async function start() {
    const chosen = preferences(nextMode);
    if (!canStart || !option || !chosen) return;
    entry.current.attempted = true;
    setCallSelection({ option, preferences: chosen });
    setBillingOpenError(false);
    await voice.start(chosen, option.voice.protocol);
  }
  /** Closing voice ends the call; accepted agent tasks keep running. */
  async function requestEnd() {
    if (endBusy.current || checkBusy.current || (uncertain && !conflict))
      return;
    if (!hasCall) {
      closeOnce();
      return;
    }
    endBusy.current = true;
    if (voice.holding) voice.hold(false);
    setHelpOpen(false);
    setEndRequested(true);
    setEndingLocal(true);
    try {
      // Keep the media owner mounted while bounded cleanup drains the call.
      // Returning to text does not prove that its remote slot has settled.
      if (conflict) await voice.endConflict();
      else await voice.end();
    } catch {
      // Recovery remains owned by the hook; a failed remote End must not trap
      // the user in voice after local audio has stopped.
    } finally {
      endBusy.current = false;
      if (mounted.current) setEndingLocal(false);
      closeOnce();
    }
  }
  async function checkStatus() {
    if (endBusy.current || checkBusy.current) return;
    checkBusy.current = true;
    setCheckingLocal(true);
    try {
      // Checking is read-only. It never substitutes for an explicit End.
      const confirmed = await voice.checkConflict();
      if (confirmed === true) closeOnce();
    } finally {
      checkBusy.current = false;
      if (mounted.current) setCheckingLocal(false);
    }
  }
  async function savePreferences(
    mode: VoicePreferences["input_mode"],
    voiceId = nextVoice,
  ) {
    const chosen = preferences(mode, voiceId);
    if (!canChangePreferences || !chosen || saveBusy.current) return;
    saveBusy.current = true;
    setSavingPreferences(true);
    setPreferencesError(false);
    try {
      await environment.api.savePreferences(chosen);
      if (!mounted.current) return;
      setModeChoice(chosen.input_mode);
      setVoiceChoice(chosen.voice ?? undefined);
      void settings.refetch();
    } catch {
      if (mounted.current) setPreferencesError(true);
    } finally {
      saveBusy.current = false;
      if (mounted.current) setSavingPreferences(false);
    }
  }
  /** Hands-free or Hold to talk, from the call itself; saved for next time. */
  function changeMode(mode: VoicePreferences["input_mode"]) {
    if (isGrok || mode === inputMode || controlsBlocked) return;
    if (voice.connected && !voice.setInputMode(mode)) return;
    if (!reducedMotion)
      LayoutAnimation.configureNext(LayoutAnimation.Presets.easeInEaseOut);
    setModeChoice(mode);
    setCallSelection((current) =>
      current
        ? {
            ...current,
            preferences: { ...current.preferences, input_mode: mode },
          }
        : current,
    );
    void savePreferences(mode);
  }
  async function openBilling() {
    setBillingOpenError(false);
    try {
      await WebBrowser.openBrowserAsync(
        new URL("/billing", environment.origin).href,
      );
    } catch {
      setBillingOpenError(true);
    }
  }
  function closePreferences() {
    pickerContext.current.allowed = false;
    setVoicePickerScope(undefined);
    onPreferencesClose?.();
  }
  const controlsBlocked =
    voice.starting ||
    confirmingEnd ||
    !!conflict ||
    uncertain ||
    checking ||
    ending;
  const returnToChat = confirmingEnd || uncertain || checking;
  const endLabel = returnToChat
    ? "Return to chat"
    : conflict
      ? "End existing call"
      : hasCall
        ? "End voice and close"
        : "Close voice";
  const holdDisabled = microphoneBlocked || !voice.connected || controlsBlocked;
  function endVoice() {
    if (returnToChat) returnToText();
    else void requestEnd();
  }
  function holdControl(compact: boolean) {
    return (
      <View style={compact ? styles.compactHoldStage : styles.holdStage}>
        <Animated.View
          testID="voice-hold-halo"
          pointerEvents="none"
          style={[
            styles.halo,
            compact && styles.compactHalo,
            {
              backgroundColor: main,
              opacity: hold.interpolate({
                inputRange: [0, 1],
                outputRange: [0, 0.22],
              }),
              transform: [
                {
                  scale: hold.interpolate({
                    inputRange: [0, 1],
                    outputRange: [1, 1.5],
                  }),
                },
              ],
            },
          ]}
        />
        <Animated.View
          style={{
            transform: [
              {
                scale: hold.interpolate({
                  inputRange: [0, 1],
                  outputRange: [1, 1.1],
                }),
              },
            ],
          }}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Hold to talk"
            accessibilityHint={voice.holding ? "Release to send" : undefined}
            accessibilityState={{
              disabled: holdDisabled,
              selected: voice.holding,
            }}
            disabled={holdDisabled}
            onPressIn={() => voice.hold(true)}
            onPressOut={() => voice.hold(false)}
            onResponderTerminate={() => voice.hold(false)}
            onAccessibilityAction={({ nativeEvent }) => {
              if (nativeEvent.actionName === "activate")
                voice.hold(!voice.holding);
            }}
            accessibilityActions={[
              {
                name: "activate",
                label: voice.holding ? "Stop talking" : "Start talking",
              },
            ]}
            style={[
              compact ? styles.compactMain : styles.main,
              {
                backgroundColor: main,
                opacity: holdDisabled ? 0.4 : 1,
              },
            ]}
          >
            <Mic size={compact ? 20 : 26} color={onMain} />
          </Pressable>
        </Animated.View>
      </View>
    );
  }
  const quiet = { color: colors.textTertiary };
  const surface = colors.segmentTrack;
  const main = colors.textPrimary;
  const onMain = colors.bg;
  const showRecovery =
    !ending && (confirmingEnd || !!conflict || uncertain || checking);
  return (
    <>
      {helpOpen && !minimized ? (
        <Modal
          visible
          transparent
          animationType={reducedMotion ? "none" : "fade"}
          onRequestClose={() => setHelpOpen(false)}
        >
          <View style={styles.helpOverlay}>
            <Pressable
              style={StyleSheet.absoluteFill}
              accessibilityRole="button"
              accessibilityLabel="Dismiss voice help"
              onPress={() => setHelpOpen(false)}
            />
            <View
              accessibilityViewIsModal
              onAccessibilityEscape={() => setHelpOpen(false)}
              onLayout={(event) =>
                setHelpHeight(event.nativeEvent.layout.height)
              }
              style={[
                styles.help,
                {
                  width: helpWidth,
                  maxHeight: height - insets.top - insets.bottom - 32,
                  top: helpTop,
                  left: Math.max(
                    insets.left + 16,
                    Math.min(
                      anchor.x + 44 - helpWidth,
                      width - insets.right - helpWidth - 16,
                    ),
                  ),
                  backgroundColor: colors.card,
                  borderColor: colors.border,
                  shadowColor: colors.shadowColor,
                },
              ]}
            >
              <View style={styles.helpHeading}>
                <Text
                  accessibilityRole="header"
                  style={[styles.helpTitle, { color: colors.textPrimary }]}
                >
                  Voice help
                </Text>
                <IconButton
                  label="Close voice help"
                  onPress={() => setHelpOpen(false)}
                >
                  <X size={18} color={colors.textPrimary} />
                </IconButton>
              </View>
              <ScrollView
                style={{ flexShrink: 1 }}
                contentContainerStyle={styles.helpContent}
              >
                <Text style={[styles.helpText, { color: colors.textPrimary }]}>
                  {inputMode === "automatic"
                    ? `Your microphone is on while unmuted. ${agentName} answers when you pause.`
                    : "Audio is sent only while you hold the microphone. Release to send."}{" "}
                  Use headphones if you hear echo.
                </Text>
                <ActionButton
                  label="Resume audio"
                  variant="secondary"
                  compact
                  disabled={!voice.connected || controlsBlocked}
                  onPress={() => {
                    setHelpOpen(false);
                    void voice.resumeAudio();
                  }}
                />
              </ScrollView>
            </View>
          </View>
        </Modal>
      ) : null}
      {voice.media ? (
        <VoiceMediaBridge
          key={thread + ":" + voice.mediaAttempt}
          media={voice.media}
          origin={environment.origin}
        />
      ) : null}
      <Animated.View
        accessibilityLabel="Voice mode"
        style={{
          transform: [
            {
              translateY: rise.interpolate({
                inputRange: [0, 1],
                outputRange: [0, 240],
              }),
            },
          ],
          opacity: rise.interpolate({
            inputRange: [0, 1],
            outputRange: [1, 0],
          }),
        }}
      >
        {/* The aura around the composer says it is in voice mode and moves
            with NyxBot's voice and yours. */}
        <VoiceComposerShell
          minimized={minimized}
          phase={phase}
          input={levels.input}
          output={levels.output}
          reducedMotion={reducedMotion}
        >
          {!minimized && (
            <ScrollView
              style={{
                maxHeight: Math.max(44, panelHeight - 24),
              }}
              keyboardShouldPersistTaps="handled"
              contentContainerStyle={{ gap: 10 }}
            >
              <VoiceComposerHeader
                name={agentName}
                status={status}
                duration={duration}
                error={!!voice.error && !voice.connected}
              >
                {option ? (
                  <View
                    accessibilityRole="radiogroup"
                    accessibilityLabel="Listening mode"
                    style={[
                      styles.modes,
                      fontScale > 1.8 && { maxWidth: "100%", flex: 1 },
                      { backgroundColor: surface },
                    ]}
                  >
                    {(isGrok
                      ? (["push_to_talk"] as const)
                      : (["automatic", "push_to_talk"] as const)
                    ).map((mode) => {
                      const selected = inputMode === mode;
                      const Icon = mode === "automatic" ? Phone : Radio;
                      const disabled = isGrok || controlsBlocked;
                      return (
                        <Pressable
                          key={mode}
                          accessibilityRole="radio"
                          accessibilityLabel={
                            mode === "automatic"
                              ? "Hands-free mode"
                              : "Hold to talk mode"
                          }
                          accessibilityHint={
                            isGrok
                              ? "This voice source uses Hold to talk"
                              : mode === "automatic"
                                ? `${agentName} listens and answers when you pause`
                                : `${agentName} listens while you hold the microphone`
                          }
                          accessibilityState={{
                            selected,
                            checked: selected,
                            disabled,
                          }}
                          disabled={disabled}
                          onPress={() => changeMode(mode)}
                          style={[
                            styles.mode,
                            selected
                              ? [
                                  styles.activeMode,
                                  { backgroundColor: colors.segmentSelected },
                                ]
                              : styles.inactiveMode,
                          ]}
                        >
                          <Icon
                            size={18}
                            color={
                              selected
                                ? colors.textPrimary
                                : colors.textSecondary
                            }
                          />
                          {selected ? (
                            <Text
                              numberOfLines={1}
                              adjustsFontSizeToFit
                              minimumFontScale={0.65}
                              style={[
                                styles.modeLabel,
                                { color: colors.textPrimary },
                              ]}
                            >
                              {mode === "automatic"
                                ? "Hands-free"
                                : "Hold to talk"}
                            </Text>
                          ) : null}
                        </Pressable>
                      );
                    })}
                  </View>
                ) : null}
                {active ? (
                  <View ref={helpAnchor} collapsable={false}>
                    <IconButton
                      label="Voice help"
                      selected={helpOpen}
                      onPress={openHelp}
                    >
                      <CircleHelp size={18} color={colors.textPrimary} />
                    </IconButton>
                  </View>
                ) : null}
              </VoiceComposerHeader>
              {preferencesOpen ? (
                <View
                  style={[
                    styles.preferences,
                    { borderColor: colors.composerBorder },
                  ]}
                >
                  <View style={styles.preferencesHeader}>
                    <Label>Voice preferences</Label>
                    <IconButton
                      label="Close voice preferences"
                      onPress={closePreferences}
                    >
                      <X size={18} color={colors.textPrimary} />
                    </IconButton>
                  </View>
                  {!currentOption ? (
                    <Label muted>Voice source unavailable</Label>
                  ) : voice.connected && currentVoice !== nextVoice ? (
                    <Label
                      muted
                    >{`Current call: ${voiceLabel ?? "Default voice"}`}</Label>
                  ) : null}
                  {option ? (
                    <SettingsRow
                      label="Voice"
                      value={
                        option.voice.voices.find((row) => row.id === nextVoice)
                          ?.label ??
                        (missingVoice ? "Unavailable" : "Choose a voice")
                      }
                      disabled={!canChangePreferences}
                      accessibilityHint="Choose the voice for your next call"
                      accessibilityState={{
                        expanded: voicePickerScope === selectionScope,
                      }}
                      onPress={() => {
                        if (!canChangePreferences || !focused) return;
                        // A held accessibility PTT toggle must not keep capturing
                        // behind a modal that hides its release control.
                        if (voice.holding) voice.hold(false);
                        setVoicePickerScope(selectionScope);
                      }}
                    />
                  ) : null}
                  <Label muted style={styles.footnote}>
                    Changes apply to your next call.
                  </Label>
                  {savingPreferences ? (
                    <ActivityIndicator
                      accessibilityLabel="Saving voice preferences"
                      color={colors.primary}
                    />
                  ) : null}
                  {preferencesError ? (
                    <Notice
                      message="Voice preferences could not be saved. Try again."
                      tone="error"
                    />
                  ) : null}
                </View>
              ) : null}
              <View style={styles.waveRow}>
                <DictationWaveform subscribe={levels.subscribeLevel} />
              </View>
              {!active ||
              captions.length > 0 ||
              showRecovery ||
              voice.error ||
              voice.recovery ||
              billingOpenError ||
              !isConnected ||
              (session?.idle_warning && !terminal && !confirmingEnd) ? (
                <View>
                  <View style={styles.transcriptFrame}>
                    <ScrollView
                      ref={transcript}
                      contentContainerStyle={styles.transcriptContent}
                      keyboardShouldPersistTaps="handled"
                      scrollEventThrottle={32}
                      onLayout={(event) =>
                        transcriptEdges.layout(event.nativeEvent.layout.height)
                      }
                      onScroll={(event) =>
                        transcriptEdges.scroll(
                          event.nativeEvent.contentOffset.y,
                        )
                      }
                      // The newest words stay in view; scroll up for the rest.
                      onContentSizeChange={(_width, height) => {
                        transcriptEdges.content(height);
                        transcript.current?.scrollToEnd?.({
                          animated: !reducedMotion,
                        });
                      }}
                    >
                      {showRecovery ? (
                        <View style={styles.recovery}>
                          <Label muted>
                            {confirmingEnd
                              ? "The previous call is finishing. You can continue with text."
                              : conflict
                                ? voice.conflictThread &&
                                  voice.conflictThread !== thread
                                  ? "A voice call is open in another chat. You can continue with text."
                                  : "A voice call is already open. You can continue with text."
                                : "Check the previous call before starting voice again. You can continue with text."}
                          </Label>
                          <View style={styles.recoveryActions}>
                            <ActionButton
                              label="Check call status"
                              variant="secondary"
                              compact
                              loading={checking}
                              disabled={ending}
                              onPress={() => void checkStatus()}
                            />
                            {!returnToChat ? (
                              <ActionButton
                                label="Return to chat"
                                variant="secondary"
                                compact
                                disabled={ending}
                                onPress={returnToText}
                              />
                            ) : null}
                          </View>
                        </View>
                      ) : null}
                      {voice.recovery ? (
                        <Card>
                          <Notice
                            message={voice.recovery.message}
                            tone="error"
                          />
                          <ActionButton
                            label="Review billing on Web"
                            variant="secondary"
                            onPress={openBilling}
                          />
                          {!hasCall ? (
                            <ActionButton
                              label="Continue with text"
                              variant="secondary"
                              onPress={closeOnce}
                            />
                          ) : null}
                          <Pressable
                            accessibilityRole="button"
                            accessibilityLabel={
                              showErrorDetails
                                ? "Hide error details"
                                : "Show error details"
                            }
                            accessibilityState={{
                              expanded: showErrorDetails,
                            }}
                            onPress={() =>
                              setShowErrorDetails((value) => !value)
                            }
                            style={styles.textButton}
                          >
                            <Label muted>
                              {showErrorDetails
                                ? "Hide error details"
                                : "Show error details"}
                            </Label>
                          </Pressable>
                          {showErrorDetails ? (
                            <Label muted>{voice.error}</Label>
                          ) : null}
                        </Card>
                      ) : voice.error ? (
                        <Notice message={voice.error} tone="error" />
                      ) : null}
                      {billingOpenError ? (
                        <Notice
                          message="Billing could not open. Visit Billing & Usage on the NyxID website to review your available credits and benefits."
                          tone="error"
                        />
                      ) : null}
                      {!isConnected ? (
                        <Notice message="You’re offline. Reconnect to start a voice call." />
                      ) : null}
                      {session?.idle_warning && !terminal && !confirmingEnd ? (
                        <Notice
                          message={
                            inputMode === "automatic"
                              ? "The call will end soon because you haven’t spoken. Say something to continue."
                              : "The call will end soon because you haven’t spoken. Hold to talk to continue."
                          }
                        />
                      ) : null}
                      {!active && !showRecovery ? (
                        <>
                          {options.isPending || settings.isPending ? (
                            <ActivityIndicator
                              accessibilityLabel="Loading voice options"
                              color={colors.primary}
                            />
                          ) : null}
                          {options.error || settings.error ? (
                            <>
                              <Notice
                                message={
                                  options.error instanceof Error
                                    ? options.error.message
                                    : settings.error instanceof Error
                                      ? settings.error.message
                                      : "Voice options could not load."
                                }
                                tone="error"
                              />
                              <ActionButton
                                label="Retry voice options"
                                variant="secondary"
                                onPress={() => {
                                  void options.refetch();
                                  void settings.refetch();
                                }}
                              />
                            </>
                          ) : null}
                          {selectionReady && saved && !option ? (
                            <Notice message="Your saved voice source is no longer available for this chat. Reconnect it or update your voice source on Web before starting a call." />
                          ) : options.isSuccess && !choices.length ? (
                            <Notice message="No voice sources are available for this chat. Voice needs an eligible connected provider or platform connection." />
                          ) : null}
                          {option && !option.available ? (
                            <Notice
                              message={
                                option.unavailable_reason ??
                                "Voice is unavailable. Reconnect your provider and try again."
                              }
                            />
                          ) : null}
                          {missingVoice ? (
                            <Notice
                              message={`Your saved voice (${nextVoice}) is no longer available. Choose a voice in Voice preferences before starting a call.`}
                            />
                          ) : null}
                        </>
                      ) : null}
                      {!active && !showRecovery && !captions.length ? (
                        <View style={styles.idle}>
                          <Text
                            style={[
                              styles.idleTitle,
                              { color: colors.textPrimary },
                            ]}
                          >
                            Hey, what can I do for you today?
                          </Text>
                          {option ? (
                            <Text
                              style={[styles.footnote, styles.center, quiet]}
                            >
                              {voiceLabel ? `${voiceLabel} voice · ` : ""}
                              {isGrok ? "Hold to talk · " : ""}
                              Change in Settings › Voice
                            </Text>
                          ) : null}
                        </View>
                      ) : null}
                      {captions.map((caption) => (
                        <Text
                          key={caption.id}
                          selectable
                          accessibilityLiveRegion="polite"
                          style={[
                            styles.liveSpeech,
                            { color: colors.textPrimary },
                          ]}
                        >
                          {caption.text}
                        </Text>
                      ))}
                    </ScrollView>
                    <ScrollEdgeFades
                      edges={transcriptEdges.edges}
                      color={colors.composerBg}
                    />
                  </View>
                </View>
              ) : null}
              {/* Agent tasks and their approvals are receipts in the chat above. */}
              <VoiceComposerControls>
                <View style={styles.sideControls}>
                  {active ? (
                    <IconButton
                      label={
                        voice.speakerMuted ? "Enable speaker" : "Mute speaker"
                      }
                      selected={voice.speakerMuted}
                      disabled={controlsBlocked}
                      onPress={() => voice.muteSpeaker(!voice.speakerMuted)}
                    >
                      {voice.speakerMuted ? (
                        <VolumeX size={18} color={colors.textPrimary} />
                      ) : (
                        <Volume2 size={18} color={colors.textPrimary} />
                      )}
                    </IconButton>
                  ) : null}
                  {active && inputMode === "push_to_talk" ? (
                    <IconButton
                      large
                      label={muted ? "Unmute microphone" : "Mute microphone"}
                      selected={muted}
                      disabled={controlsBlocked}
                      onPress={() => void voice.mute(!muted)}
                    >
                      {/* Always the mute symbol, so it never reads as a second mic. */}
                      <MicOff size={20} color={colors.textPrimary} />
                    </IconButton>
                  ) : null}
                </View>
                {!active ? (
                  <View style={styles.holdColumn}>
                    <MainButton
                      label={
                        needsRetry
                          ? "Try again"
                          : status === "Connecting…"
                            ? "Connecting voice"
                            : status
                      }
                      fill={main}
                      disabled={!canStart || !needsRetry}
                      onPress={() => void start()}
                    >
                      {ending || checking ? (
                        <ActivityIndicator color={onMain} />
                      ) : (
                        <Mic size={26} color={onMain} />
                      )}
                    </MainButton>
                    {/* The hint never claims a connection the status rules out. */}
                    <Text
                      style={[styles.holdHint, { color: colors.textSecondary }]}
                    >
                      {ending
                        ? "Ending…"
                        : needsRetry
                          ? "Try again"
                          : status === "Connecting…"
                            ? "Connecting…"
                            : ""}
                    </Text>
                  </View>
                ) : inputMode === "push_to_talk" ? (
                  <View style={styles.holdColumn}>
                    {holdControl(false)}
                    {/* The instruction sits under the microphone it describes. */}
                    <Text
                      accessibilityElementsHidden
                      importantForAccessibility="no"
                      style={[styles.holdHint, { color: colors.textSecondary }]}
                    >
                      {microphoneBlocked && voice.connected
                        ? "Muted"
                        : voice.holding
                          ? "Release to send"
                          : "Hold to talk"}
                    </Text>
                  </View>
                ) : (
                  <View style={styles.holdColumn}>
                    <MainButton
                      label={muted ? "Unmute microphone" : "Mute microphone"}
                      fill={muted ? colors.danger : main}
                      disabled={controlsBlocked}
                      onPress={() => void voice.mute(!muted)}
                    >
                      {muted ? (
                        <MicOff size={26} color={colors.onPrimary} />
                      ) : (
                        <Mic size={26} color={onMain} />
                      )}
                    </MainButton>
                    {muted ? (
                      <Text
                        accessibilityElementsHidden
                        importantForAccessibility="no"
                        style={[
                          styles.holdHint,
                          { color: colors.textSecondary },
                        ]}
                      >
                        Muted
                      </Text>
                    ) : null}
                  </View>
                )}
                <View style={styles.endControl}>
                  <IconButton
                    large
                    label={endLabel}
                    disabled={ending}
                    onPress={endVoice}
                  >
                    {hasCall && !returnToChat && inputMode === "automatic" ? (
                      <PhoneOff size={20} color={colors.danger} />
                    ) : (
                      <X
                        size={20}
                        color={
                          hasCall && !returnToChat
                            ? colors.danger
                            : colors.textPrimary
                        }
                      />
                    )}
                  </IconButton>
                </View>
              </VoiceComposerControls>
            </ScrollView>
          )}
          {minimized && (
            <VoiceComposerCompactRow
              status={status}
              duration={duration}
              liveText={captions.map((caption) => caption.text).join(" ")}
              subscribeLevel={levels.subscribeLevel}
              instruction={
                inputMode === "push_to_talk" && active
                  ? voice.holding
                    ? "Release to send"
                    : "Hold to talk"
                  : undefined
              }
              controls={
                active && inputMode === "push_to_talk"
                  ? holdControl(true)
                  : undefined
              }
              onExpand={onExpand}
              onClose={endVoice}
            />
          )}
        </VoiceComposerShell>
      </Animated.View>
      {preferencesOpen &&
      !minimized &&
      focused &&
      option &&
      voicePickerScope === selectionScope ? (
        <WorkspaceSelectionSheet
          key={selectionScope}
          scopeKey={selectionScope}
          label="Voice"
          title="Choose voice"
          description="Changes apply to your next call."
          options={option.voice.voices.map((row) => ({
            value: row.id,
            label: row.label,
          }))}
          value={nextVoice ? [nextVoice] : []}
          disabled={!canChangePreferences}
          onChange={(values) => {
            if (
              !mounted.current ||
              !pickerContext.current.allowed ||
              pickerContext.current.scope !== selectionScope ||
              values.length !== 1
            )
              return;
            void savePreferences(nextMode, values[0]);
          }}
          onClose={() => {
            pickerContext.current.allowed = false;
            setVoicePickerScope(undefined);
          }}
        />
      ) : null}
    </>
  );
}

const styles = StyleSheet.create({
  recovery: { gap: 10 },
  recoveryActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  preferences: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 8,
    gap: 6,
  },
  preferencesHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  center: { textAlign: "center" },
  frame: { gap: 10, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 10 },
  waveRow: { height: 22, flexDirection: "row" },
  liveSpeech: { fontFamily: fonts.bodyRegular, fontSize: 18, lineHeight: 26 },
  idle: { alignItems: "center", gap: 8 },
  idleTitle: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    textAlign: "center",
  },
  helpOverlay: { flex: 1 },
  help: {
    position: "absolute",
    borderRadius: 16,
    borderWidth: 1,
    padding: 12,
    gap: 8,
    shadowOpacity: 0.15,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 4 },
    elevation: 8,
  },
  helpHeading: { flexDirection: "row", alignItems: "center", gap: 8 },
  helpTitle: {
    flex: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 15,
    lineHeight: 20,
  },
  helpContent: { gap: 12 },
  helpText: { fontFamily: fonts.body, fontSize: 14, lineHeight: 20 },
  transcriptFrame: { position: "relative" },
  transcriptContent: { gap: 10, paddingHorizontal: 2, paddingBottom: 2 },
  textButton: { minHeight: 32, justifyContent: "center", alignSelf: "center" },
  footnote: { fontFamily: fonts.body, fontSize: 12, lineHeight: 17 },
  modes: {
    flexDirection: "row",
    flexShrink: 1,
    maxWidth: "70%",
    borderRadius: 18,
    padding: 3,
  },
  mode: {
    minHeight: 44,
    minWidth: 44,
    flexDirection: "row",
    gap: 6,
    paddingHorizontal: 10,
    borderRadius: 15,
    alignItems: "center",
    justifyContent: "center",
  },
  activeMode: { flexShrink: 1 },
  inactiveMode: { width: 44, paddingHorizontal: 0 },
  modeLabel: {
    flexShrink: 1,
    fontFamily: fonts.bodySemi,
    fontSize: 13,
    lineHeight: 18,
  },
  sideControls: { flex: 1, flexDirection: "row", alignItems: "center", gap: 4 },
  endControl: { flex: 1, alignItems: "flex-end" },
  main: {
    width: 64,
    height: 64,
    borderRadius: 32,
    alignItems: "center",
    justifyContent: "center",
  },
  holdColumn: { alignItems: "center", gap: 6 },
  holdStage: {
    width: 64,
    height: 64,
    alignItems: "center",
    justifyContent: "center",
  },
  compactMain: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  compactHoldStage: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  compactHalo: { width: 44, height: 44, borderRadius: 22 },
  halo: { position: "absolute", width: 64, height: 64, borderRadius: 32 },
  holdHint: {
    fontFamily: fonts.bodySemi,
    fontSize: 12,
    lineHeight: 16,
    minHeight: 16,
  },
});
