import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Reanimated, {
  useAnimatedStyle,
  type SharedValue,
} from "react-native-reanimated";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import { Ellipsis, Mic, MicOff, PhoneOff, X } from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../../components/workspace/WorkspaceUI";
import { SettingsRow } from "../../../components/settings/SettingsList";
import { WorkspaceSelectionSheet } from "../../../components/workspace/WorkspaceSelectionField";
import { useReducedMotion } from "../../../lib/accessibility/useReducedMotion";
import { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { useNetworkStatus } from "../../../hooks/useNetworkStatus";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { ChatComposerFrame } from "../ChatComposer";
import { useNativeVoice } from "./useNativeVoice";
import { useVoiceLevels } from "./useVoiceLevels";
import { VoiceMediaBridge } from "./VoiceMediaBridge";
import {
  chooseVoiceOption,
  voiceElapsed,
  voiceOptionKey,
  voiceTariff,
} from "./voiceSelection";
import type {
  VoiceOption,
  VoicePreferences,
  VoiceSnapshot,
} from "./voiceContracts";

type CaptureState = { listening: boolean; inputActive: boolean };
type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  onUpdate: () => void;
  /** Scrolls the main chat to the approval; it does not stop the call or task. */
  onReview: (acknowledgementId?: string) => void;
  /** The main chat is the only transcript surface. */
  onSnapshot?: (snapshot: VoiceSnapshot | undefined) => void;
  onCaptureState?: (state: CaptureState) => void;
  /** Controlled by the main chat's top-right Voice preferences action. */
  preferencesOpen?: boolean;
  onPreferencesClose?: () => void;
};
type Task = VoiceSnapshot["tasks"][number];
type CallSelection = { option: VoiceOption; preferences: VoicePreferences };

const TASK_STATE: Record<Task["state"], string> = {
  queued: "Queued",
  claimed: "Running",
  awaiting_confirmation: "Needs your OK",
  completed: "Done",
  cancelled: "Ended",
};

function taskState(task: Task) {
  if (task.state !== "cancelled") return TASK_STATE[task.state];
  if (task.outcome === "failed") return "Failed";
  if (task.outcome === "cancelled") return "Stopped";
  return TASK_STATE.cancelled;
}

/** Voice replaces the text composer; the main chat keeps all words and cards. */
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
  onReview,
  onSnapshot,
  onCaptureState,
  preferencesOpen,
  onPreferencesClose,
  environment,
}: Props & { environment: ReturnType<typeof nativeVoiceEnvironment> }) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const levels = useVoiceLevels(voice.media);
  const [now, setNow] = useState(Date.now());
  const [tasksOpen, setTasksOpen] = useState(false);
  const [taskDetailsId, setTaskDetailsId] = useState<string>();
  const [localPreferencesOpen, setLocalPreferencesOpen] = useState(false);
  const [voicePickerScope, setVoicePickerScope] = useState<string>();
  const [pricingOpen, setPricingOpen] = useState(false);
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
  const [inputActive, setInputActive] = useState(false);
  const entry = useRef({ thread, epoch: environment.epoch, attempted: false });
  const closed = useRef(false);
  const mounted = useRef(true);
  const endBusy = useRef(false);
  const checkBusy = useRef(false);
  const saveBusy = useRef(false);
  const callbacks = useRef({ onSnapshot, onCaptureState });
  callbacks.current = { onSnapshot, onCaptureState };
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
  // This flag comes from the client's actual setInput decision, not captions.
  const listening =
    !!voice.inputEnabled &&
    voice.connected &&
    !microphoneBlocked &&
    !conflict &&
    !uncertain;
  const outputActive =
    voice.connected && levels.speaking && !voice.speakerMuted && !confirmingEnd;
  const tasks = voice.snapshot?.tasks ?? [];
  const needsOk = tasks.some((task) => task.state === "awaiting_confirmation");
  const running = tasks.some(
    (task) => task.state === "queued" || task.state === "claimed",
  );
  const pendingTasks = tasks.filter(
    (task) => task.state === "queued" || task.state === "claimed",
  ).length;
  const failedTasks = tasks.filter(
    (task) => task.state === "cancelled" && task.outcome === "failed",
  ).length;
  const taskActivity = needsOk
    ? "Needs your OK"
    : pendingTasks
      ? `${pendingTasks} ${pendingTasks === 1 ? "task" : "tasks"} in progress`
      : failedTasks
        ? `${failedTasks} ${failedTasks === 1 ? "task" : "tasks"} failed`
        : "Task activity";
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
    !active &&
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
  const preferencesVisible = preferencesOpen ?? localPreferencesOpen;
  const selectionScope = JSON.stringify([
    environment.epoch,
    thread,
    option ? voiceOptionKey(option) : null,
  ]);
  const pickerContext = useRef({ scope: selectionScope, allowed: false });
  pickerContext.current = {
    scope: selectionScope,
    allowed:
      preferencesVisible &&
      focused &&
      canChangePreferences &&
      voicePickerScope === selectionScope,
  };
  useEffect(() => {
    if (!preferencesVisible || !focused) setVoicePickerScope(undefined);
  }, [preferencesVisible, focused]);
  const duration = voiceElapsed(session?.created_at, session?.closed_at, now);
  const status = checking
    ? "Checking call status…"
    : ending
      ? "Ending…"
      : confirmingEnd
        ? "Confirming call ended…"
        : uncertain
          ? "Call status unconfirmed"
          : conflict
            ? "Call already active"
            : !isConnected
              ? "Offline"
              : voice.starting
                ? "Connecting…"
                : voice.connected
                  ? outputActive
                    ? "Assistant is speaking"
                    : muted
                      ? "Microphone off"
                      : listening
                        ? "Listening"
                        : inputMode === "push_to_talk" && !microphoneBlocked
                          ? "Hold to talk"
                          : "Microphone paused"
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
                          : selectionReady
                            ? "Connecting…"
                            : "Preparing voice…";

  useEffect(() => {
    mounted.current = true;
    closed.current = false;
    return () => {
      mounted.current = false;
      callbacks.current.onSnapshot?.(undefined);
      callbacks.current.onCaptureState?.({
        listening: false,
        inputActive: false,
      });
    };
  }, [thread, environment]);
  useEffect(() => {
    callbacks.current.onSnapshot?.(voice.snapshot ?? undefined);
  }, [voice.snapshot]);
  useEffect(() => {
    callbacks.current.onCaptureState?.({
      listening,
      inputActive: listening && inputActive,
    });
  }, [listening, inputActive]);
  useEffect(() => {
    setInputActive(false);
    if (!listening || !voice.media) return;
    let release: ReturnType<typeof setTimeout> | undefined;
    const unsubscribe = voice.media.subscribe((event) => {
      if (event.type !== "level") return;
      clearTimeout(release);
      if (event.input >= 0.08) {
        setInputActive(true);
        release = setTimeout(() => setInputActive(false), 750);
      } else {
        release = setTimeout(() => setInputActive(false), 250);
      }
    });
    return () => {
      unsubscribe();
      clearTimeout(release);
    };
  }, [voice.media, listening]);
  useEffect(() => {
    if (!active && !confirmingEnd) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [active, confirmingEnd]);
  useEffect(() => {
    setShowErrorDetails(false);
  }, [voice.error]);
  useEffect(() => {
    // A mounted live call also keeps its original mode and voice while the
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
    if (!isConnected && hasCall) void requestEnd();
  }, [isConnected]);
  useEffect(() => {
    if (!focused && hasCall) void requestEnd();
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

  function closeOnce() {
    if (!mounted.current || closed.current) return;
    closed.current = true;
    onClose();
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
  async function requestEnd() {
    if (endBusy.current || checkBusy.current || (uncertain && !conflict))
      return;
    if (!hasCall) {
      closeOnce();
      return;
    }
    endBusy.current = true;
    if (voice.holding) voice.hold(false);
    setEndRequested(true);
    setEndingLocal(true);
    try {
      const confirmed = conflict
        ? await voice.endConflict()
        : await voice.end();
      if (confirmed === true) closeOnce();
    } finally {
      endBusy.current = false;
      if (mounted.current) setEndingLocal(false);
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
  function review(acknowledgementId?: string) {
    if (voice.holding) voice.hold(false);
    onReview(acknowledgementId);
  }
  function closePreferences() {
    pickerContext.current.allowed = false;
    setVoicePickerScope(undefined);
    setLocalPreferencesOpen(false);
    onPreferencesClose?.();
  }
  function taskRow(task: Task, index: number) {
    const detailsOpen = taskDetailsId === task.id;
    const hasDetails = !!(task.title?.trim() || task.progress?.detail.trim());
    return (
      <View key={task.id} style={styles.taskRow}>
        <View style={styles.taskHeader}>
          <Label style={styles.fill}>
            {`Task ${index + 1} · ${taskState(task)}`}
          </Label>
          <View style={styles.taskActions}>
            {hasDetails ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`${detailsOpen ? "Hide" : "Show"} details for task ${index + 1}`}
                accessibilityState={{ expanded: detailsOpen }}
                onPress={() => {
                  if (voice.holding) voice.hold(false);
                  setTaskDetailsId(detailsOpen ? undefined : task.id);
                }}
                style={styles.taskDetailsButton}
              >
                <Label muted>{detailsOpen ? "Hide details" : "Details"}</Label>
              </Pressable>
            ) : null}
            {task.state === "awaiting_confirmation" ? (
              <ActionButton
                label="Review in chat"
                variant="secondary"
                compact
                onPress={() => review(task.pending_acknowledgement_ids[0])}
              />
            ) : null}
            {task.state === "queued" || task.state === "claimed" ? (
              <ActionButton
                label="Stop task"
                variant="secondary"
                compact
                onPress={() => void voice.stopTask(task.id)}
              />
            ) : null}
          </View>
        </View>
        {detailsOpen ? (
          <View style={styles.taskDetails}>
            {task.title?.trim() ? (
              <>
                <Label muted style={styles.taskState}>
                  Request from voice assistant
                </Label>
                <Label selectable>{task.title}</Label>
              </>
            ) : null}
            {task.progress?.detail.trim() ? (
              <Label muted selectable style={styles.taskState}>
                {task.progress.detail}
              </Label>
            ) : null}
          </View>
        ) : null}
      </View>
    );
  }

  const quiet = { color: colors.textTertiary };
  const controlsBlocked =
    voice.starting ||
    confirmingEnd ||
    !!conflict ||
    uncertain ||
    checking ||
    ending;
  const endDisabled = ending || checking || (uncertain && !conflict);
  return (
    <>
      {voice.media ? (
        <VoiceMediaBridge
          key={thread + ":" + voice.mediaAttempt}
          media={voice.media}
          origin={environment.origin}
        />
      ) : null}
      <ChatComposerFrame style={styles.frame} bounce={false}>
        <View
          accessibilityLabel={`Voice conversation with ${agentName}`}
          style={styles.bar}
        >
          <View style={styles.identity}>
            {voice.starting || checking || ending ? (
              <ActivityIndicator size="small" color={colors.primary} />
            ) : (
              <VoiceWave
                input={levels.input}
                output={levels.output}
                speaking={outputActive}
                quiet={!listening && !outputActive}
                reducedMotion={reducedMotion}
              />
            )}
            <View style={styles.stateText}>
              <Text
                accessibilityLiveRegion="polite"
                style={[styles.status, { color: colors.textPrimary }]}
              >
                {status}
              </Text>
              {session ? (
                <Text
                  accessibilityLabel={`Call time ${duration}`}
                  style={[styles.elapsed, quiet]}
                >
                  {duration}
                  {muted && outputActive ? " · Microphone off" : ""}
                </Text>
              ) : null}
            </View>
          </View>
          {voice.connected && !controlsBlocked ? (
            inputMode === "push_to_talk" && !muted ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Hold to talk"
                accessibilityHint="Press and hold to speak; release to stop sending audio"
                accessibilityState={{
                  disabled: microphoneBlocked,
                  selected: voice.holding,
                }}
                accessibilityActions={[
                  {
                    name: "activate",
                    label: voice.holding ? "Stop talking" : "Start talking",
                  },
                ]}
                disabled={microphoneBlocked}
                onPressIn={() => voice.hold(true)}
                onPressOut={() => voice.hold(false)}
                onResponderTerminate={() => voice.hold(false)}
                onAccessibilityAction={(event) => {
                  if (event.nativeEvent.actionName === "activate")
                    voice.hold(!voice.holding);
                }}
                style={[
                  styles.hold,
                  {
                    backgroundColor: voice.holding
                      ? colors.primaryTone.bg
                      : colors.segmentTrack,
                    opacity: microphoneBlocked ? 0.4 : 1,
                  },
                ]}
              >
                <Mic size={17} color={colors.textPrimary} />
                <Text
                  style={[styles.controlText, { color: colors.textPrimary }]}
                >
                  {voice.holding ? "Release" : "Hold to talk"}
                </Text>
              </Pressable>
            ) : (
              <IconButton
                label={muted ? "Unmute microphone" : "Mute microphone"}
                selected={muted}
                onPress={() => void voice.mute(!muted)}
              >
                {muted ? (
                  <MicOff size={20} color={colors.danger} />
                ) : (
                  <Mic size={20} color={colors.textPrimary} />
                )}
              </IconButton>
            )
          ) : null}
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={
              confirmingEnd
                ? "Request end again"
                : conflict
                  ? "End existing call"
                  : hasCall
                    ? "End voice"
                    : "Return to chat"
            }
            accessibilityState={{ disabled: endDisabled }}
            disabled={endDisabled}
            onPress={() => void requestEnd()}
            style={[
              styles.end,
              {
                backgroundColor: colors.textPrimary,
                opacity: endDisabled ? 0.45 : 1,
              },
            ]}
          >
            <PhoneOff size={17} color={colors.bg} />
            <Text style={[styles.controlText, { color: colors.bg }]}>
              {confirmingEnd ? "End again" : hasCall ? "End" : "Close"}
            </Text>
          </Pressable>
        </View>
        {preferencesOpen === undefined ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Voice preferences"
            accessibilityState={{ expanded: preferencesVisible }}
            onPress={() => setLocalPreferencesOpen((value) => !value)}
            style={styles.preferencesFallback}
          >
            <Ellipsis size={18} color={colors.textSecondary} />
          </Pressable>
        ) : null}
        {confirmingEnd || conflict || uncertain ? (
          <View style={styles.recovery}>
            <Label muted>
              {confirmingEnd
                ? "Your microphone is off. Wait for the call to finish before starting another."
                : conflict
                  ? voice.conflictThread && voice.conflictThread !== thread
                    ? "A voice call is open in another chat. End it explicitly or check its status."
                    : "A voice call is already open. End it explicitly or check its status."
                  : "The existing call could not be located. Check call status before trying again."}
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
            </View>
          </View>
        ) : null}
        {voice.recovery ? (
          <Card>
            <Notice message={voice.recovery.message} tone="error" />
            <View style={styles.recoveryActions}>
              <ActionButton
                label="Review billing on Web"
                variant="secondary"
                compact
                onPress={openBilling}
              />
              {!hasCall ? (
                <ActionButton
                  label="Continue with text"
                  variant="secondary"
                  compact
                  onPress={closeOnce}
                />
              ) : null}
            </View>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={
                showErrorDetails ? "Hide error details" : "Show error details"
              }
              accessibilityState={{ expanded: showErrorDetails }}
              onPress={() => setShowErrorDetails((value) => !value)}
              style={styles.textButton}
            >
              <Label muted>
                {showErrorDetails ? "Hide error details" : "Show error details"}
              </Label>
            </Pressable>
            {showErrorDetails ? <Label muted>{voice.error}</Label> : null}
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
        {!active && !confirmingEnd && !conflict && !uncertain ? (
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
            {voice.error && canStart ? (
              <ActionButton
                label="Try again"
                variant="secondary"
                compact
                onPress={() => void start()}
              />
            ) : null}
          </>
        ) : null}
        {tasks.length ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={
              tasksOpen ? "Hide agent tasks" : "Show agent tasks"
            }
            accessibilityState={{ expanded: tasksOpen }}
            onPress={() => {
              if (voice.holding) voice.hold(false);
              setTaskDetailsId(undefined);
              setTasksOpen((value) => !value);
            }}
            style={[
              styles.taskChip,
              {
                backgroundColor: needsOk
                  ? colors.warningTone.bg
                  : !running && failedTasks
                    ? colors.dangerTone.bg
                    : colors.segmentTrack,
              },
            ]}
          >
            {running ? (
              <ActivityIndicator size="small" color={colors.textSecondary} />
            ) : null}
            <Text
              style={[
                styles.controlText,
                {
                  color: needsOk
                    ? colors.warningTone.text
                    : !running && failedTasks
                      ? colors.dangerTone.text
                      : colors.textPrimary,
                },
              ]}
            >
              {taskActivity}
            </Text>
          </Pressable>
        ) : null}
        {tasksOpen && tasks.length ? (
          <ScrollView
            accessibilityLabel="Agent tasks"
            style={[styles.tray, { backgroundColor: colors.segmentTrack }]}
            contentContainerStyle={styles.trayContent}
          >
            <Label>Task activity</Label>
            {tasks.map(taskRow)}
            {running || needsOk ? (
              <Label muted style={styles.footnote}>
                {running
                  ? "Tasks can continue after the call ends. Stop a task to cancel it."
                  : "Review the pending request in chat to continue."}
              </Label>
            ) : null}
          </ScrollView>
        ) : null}
        {preferencesVisible ? (
          <View
            style={[styles.preferences, { borderColor: colors.composerBorder }]}
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
            <ScrollView
              accessibilityLabel="Voice preferences"
              style={styles.preferencesScroll}
              contentContainerStyle={styles.trayContent}
            >
              {!currentOption ? (
                <Label muted>Voice source unavailable</Label>
              ) : voice.connected && currentVoice !== nextVoice ? (
                <Label
                  muted
                >{`Current call: ${voiceLabel ?? "Default voice"}`}</Label>
              ) : null}
              <Label muted style={styles.footnote}>
                Changes apply to your next call.
              </Label>
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
              {option && !isGrok ? (
                <View
                  accessibilityRole="radiogroup"
                  accessibilityLabel="Listening mode for next call"
                  style={styles.preferencesChoices}
                >
                  {(["automatic", "push_to_talk"] as const).map((mode) => (
                    <Choice
                      key={mode}
                      label={
                        mode === "automatic"
                          ? "Hands-free mode"
                          : "Hold to talk mode"
                      }
                      selected={nextMode === mode}
                      disabled={!canChangePreferences}
                      onPress={() => void savePreferences(mode)}
                    />
                  ))}
                </View>
              ) : isGrok ? (
                <Label muted>Grok supports Hold to talk.</Label>
              ) : null}
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
              {voice.connected ? (
                <View style={styles.recoveryActions}>
                  <ActionButton
                    label={
                      voice.speakerMuted ? "Enable speaker" : "Mute speaker"
                    }
                    variant="secondary"
                    compact
                    onPress={() => voice.muteSpeaker(!voice.speakerMuted)}
                  />
                  <ActionButton
                    label="No sound? Resume audio"
                    variant="secondary"
                    compact
                    onPress={() => void voice.resumeAudio()}
                  />
                </View>
              ) : null}
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={
                  pricingOpen ? "Hide voice pricing" : "View voice pricing"
                }
                accessibilityState={{ expanded: pricingOpen }}
                onPress={() => setPricingOpen((value) => !value)}
                style={styles.textButton}
              >
                <Label muted>
                  {pricingOpen ? "Hide voice pricing" : "View voice pricing"}
                </Label>
              </Pressable>
              {pricingOpen && currentOption ? (
                <Label muted style={styles.footnote}>
                  {voiceTariff(currentOption)}
                </Label>
              ) : null}
            </ScrollView>
          </View>
        ) : null}
      </ChatComposerFrame>
      {preferencesVisible &&
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

function IconButton({
  label,
  selected = false,
  onPress,
  children,
}: {
  label: string;
  selected?: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ selected }}
      onPress={onPress}
      style={[
        styles.icon,
        {
          backgroundColor: selected
            ? (colors.dangerTone?.bg ?? colors.segmentTrack)
            : colors.segmentTrack,
        },
      ]}
    >
      {children}
    </Pressable>
  );
}
function Choice({
  label,
  selected,
  disabled,
  onPress,
}: {
  label: string;
  selected: boolean;
  disabled: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="radio"
      accessibilityLabel={label}
      accessibilityState={{ checked: selected, selected, disabled }}
      disabled={disabled}
      onPress={onPress}
      style={[
        styles.choice,
        {
          backgroundColor: selected
            ? colors.segmentSelected
            : colors.segmentTrack,
          opacity: disabled ? 0.45 : 1,
        },
      ]}
    >
      <Text style={[styles.controlText, { color: colors.textPrimary }]}>
        {label.replace(" mode", "")}
      </Text>
    </Pressable>
  );
}
function VoiceWave({
  input,
  output,
  speaking,
  quiet,
  reducedMotion,
}: {
  input: SharedValue<number>;
  output: SharedValue<number>;
  speaking: boolean;
  quiet: boolean;
  reducedMotion: boolean;
}) {
  const { colors } = useTheme();
  return (
    <View
      accessibilityElementsHidden
      importantForAccessibility="no"
      style={styles.wave}
    >
      {[8, 16, 24, 16, 8].map((height, index) => (
        <VoiceWaveBar
          key={index}
          height={height}
          level={speaking ? output : input}
          quiet={quiet}
          reducedMotion={reducedMotion}
          color={quiet ? colors.textTertiary : colors.primary}
        />
      ))}
    </View>
  );
}
function VoiceWaveBar({
  height,
  level,
  quiet,
  reducedMotion,
  color,
}: {
  height: number;
  level: SharedValue<number>;
  quiet: boolean;
  reducedMotion: boolean;
  color: string;
}) {
  const animated = useAnimatedStyle(() => ({
    transform: [
      {
        scaleY:
          quiet || reducedMotion
            ? 1
            : 0.55 + Math.min(1, Math.max(0, level.value)) * 0.9,
      },
    ],
  }));
  return (
    <Reanimated.View
      style={[
        styles.waveBar,
        { height: quiet ? 4 : height, backgroundColor: color },
        animated,
      ]}
    />
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  frame: { paddingHorizontal: 14, paddingVertical: 12, gap: 10 },
  bar: { flexDirection: "row", alignItems: "center", minHeight: 48, gap: 10 },
  identity: {
    flex: 1,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: 11,
  },
  stateText: { flex: 1, minWidth: 0 },
  status: { fontFamily: fonts.bodySemi, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 11, lineHeight: 16 },
  icon: {
    width: 44,
    height: 44,
    borderRadius: 22,
    alignItems: "center",
    justifyContent: "center",
  },
  end: {
    minHeight: 44,
    borderRadius: 22,
    paddingHorizontal: 12,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 5,
  },
  hold: {
    minHeight: 44,
    borderRadius: 22,
    paddingHorizontal: 11,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 5,
  },
  controlText: { fontFamily: fonts.bodySemi, fontSize: 12, lineHeight: 17 },
  preferencesFallback: {
    minHeight: 32,
    alignSelf: "flex-end",
    justifyContent: "center",
    paddingHorizontal: 10,
  },
  recovery: { gap: 10 },
  recoveryActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  taskChip: {
    alignSelf: "flex-start",
    minHeight: 44,
    borderRadius: 16,
    paddingHorizontal: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 7,
  },
  tray: { maxHeight: 200, borderRadius: 14 },
  trayContent: { gap: 10, padding: 8 },
  taskRow: { gap: 6 },
  taskHeader: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 8,
  },
  taskActions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  taskDetailsButton: {
    minHeight: 44,
    paddingHorizontal: 8,
    justifyContent: "center",
  },
  taskDetails: { gap: 4 },
  taskState: { fontSize: 12 },
  preferences: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 8,
    gap: 4,
  },
  preferencesHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  preferencesScroll: { maxHeight: 260 },
  preferencesChoices: { flexDirection: "row", flexWrap: "wrap", gap: 7 },
  choice: {
    minHeight: 44,
    borderRadius: 15,
    justifyContent: "center",
    paddingHorizontal: 12,
  },
  textButton: { minHeight: 44, justifyContent: "center" },
  footnote: { fontFamily: fonts.body, fontSize: 12, lineHeight: 17 },
  wave: {
    width: 32,
    height: 32,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
    gap: 3,
  },
  waveBar: { width: 3, borderRadius: 2 },
});
