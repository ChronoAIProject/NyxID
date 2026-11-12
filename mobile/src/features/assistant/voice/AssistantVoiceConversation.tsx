import { useEffect, useRef, useState, type ReactNode } from "react";
import { useAppLanguage } from "../../../i18n/LanguageProvider";
import {
  ActivityIndicator,
  AppState,
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
import { withVoiceRequestDeadline } from "./voiceRequestDeadline";
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
  onReview?: (acknowledgementId?: string) => void;
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

async function loadVoiceSelection<T>(
  signal: AbortSignal,
  operation: (signal: AbortSignal) => Promise<T>,
) {
  const controller = new AbortController();
  const cancel = () => controller.abort(signal.reason);
  if (signal.aborted) cancel();
  else signal.addEventListener("abort", cancel, { once: true });
  try {
    return await withVoiceRequestDeadline(controller, () =>
      operation(controller.signal),
    );
  } catch (cause) {
    if (
      cause &&
      typeof cause === "object" &&
      "name" in cause &&
      cause.name === "VoiceRequestTimeoutError"
    )
      throw new Error("Voice options could not load.");
    throw cause;
  } finally {
    signal.removeEventListener("abort", cancel);
  }
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
}: Props & {
  environment: ReturnType<typeof nativeVoiceEnvironment>;
}) {
  const { colors } = useTheme();
  const { t } = useAppLanguage();
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const levels = useVoiceLevels(voice.media);
  const [tasksOpen, setTasksOpen] = useState(false);
  const [taskDetailsId, setTaskDetailsId] = useState<string>();
  const [localPreferencesOpen, setLocalPreferencesOpen] = useState(false);
  const [pricingOpen, setPricingOpen] = useState(false);
  const [now, setNow] = useState(Date.now());
  const [voicePickerScope, setVoicePickerScope] = useState<string>();
  const voicePickerContent = useRef<{
    options: { value: string; label: string }[];
    value: string[];
  }>({ options: [], value: [] });
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
  type Restart = {
    scope: string;
    sessionId: string;
    thread: string;
    phase: "disconnecting" | "starting";
  };
  const [restart, setRestart] = useState<Restart>();
  const restartIntent = useRef<Restart | undefined>(undefined);
  const restartConsidered = useRef(false);
  const [restartExpired, setRestartExpired] = useState(false);
  const [restartNotice, setRestartNotice] = useState<string>();
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
    queryFn: ({ signal }) =>
      loadVoiceSelection(signal, (requestSignal) =>
        environment.api.options(thread, requestSignal),
      ),
    enabled: focused && isConnected,
    retry: false,
  });
  const settings = useQuery({
    queryKey: ["workspace", "voice-settings", environment.epoch],
    queryFn: ({ signal }) =>
      loadVoiceSelection(signal, (requestSignal) =>
        environment.api.settings(requestSignal),
      ),
    enabled: focused && isConnected,
    retry: false,
  });
  const saved = settings.data?.voice;
  const choices = options.data?.options ?? [];
  const selectionReady = options.isSuccess && settings.isSuccess;
  const cachedOption =
    options.data && settings.data
      ? chooseVoiceOption(choices, saved)
      : undefined;
  const option = selectionReady ? cachedOption : undefined;
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
  // This flag comes from the client's actual setInput decision, not captions.
  const listening =
    !!voice.inputEnabled &&
    focused &&
    isConnected &&
    voice.connected &&
    !microphoneBlocked &&
    !conflict &&
    !uncertain;
  const outputActive =
    voice.connected && levels.speaking && !voice.speakerMuted && !confirmingEnd;
  const tasks = voice.snapshot?.tasks ?? [];
  const needsOk = tasks.some((task) => task.state === "awaiting_confirmation");
  const pendingTasks = tasks.filter(
    (task) => task.state === "queued" || task.state === "claimed",
  ).length;
  const running = pendingTasks > 0;
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
    cachedOption ? voiceOptionKey(cachedOption) : null,
  ]);
  const restartScope = JSON.stringify([selectionScope, nextVoice, nextMode]);
  const restarting = restart?.scope === restartScope;
  const canReplaceCall =
    !!conflict &&
    !uncertain &&
    !checking &&
    !ending &&
    canChangePreferences &&
    !missingVoice &&
    focused;
  const restartContext = useRef({
    scope: restartScope,
    allowed: canReplaceCall,
  });
  restartContext.current = { scope: restartScope, allowed: canReplaceCall };
  const pickerContext = useRef({ scope: selectionScope, allowed: false });
  pickerContext.current = {
    scope: selectionScope,
    allowed:
      preferencesVisible &&
      focused &&
      canChangePreferences &&
      voicePickerScope === selectionScope,
  };
  const status = restarting
    ? restart.phase === "starting"
      ? "Starting new call…"
      : "Disconnecting previous call…"
    : checking
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
    let current = true;
    let quiet = false;
    const releaseInput = () => {
      release = undefined;
      if (current) setInputActive(false);
    };
    const unsubscribe = voice.media.subscribe((event) => {
      if (!current || event.type !== "level") return;
      if (event.input >= 0.08) {
        quiet = false;
        clearTimeout(release);
        setInputActive(true);
        release = setTimeout(releaseInput, 750);
      } else if (!quiet) {
        // Frequent silent native samples must not postpone the first release.
        quiet = true;
        clearTimeout(release);
        release = setTimeout(releaseInput, 250);
      }
    });
    return () => {
      current = false;
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
    // A mounted live call also keeps its original source and voice while the
    // user edits next-call preferences or the settings query refreshes.
    if (active && !callSelection && option) {
      const chosen = preferences(nextMode);
      if (chosen) setCallSelection({ option, preferences: chosen });
    }
  }, [active, callSelection, option, nextMode]);
  useEffect(() => {
    // Entering voice may wait for this chat's already requested End. An active
    // call, unknown slot, or previous failed attempt requires an explicit action.
    if (!restartConsidered.current && conflict) {
      if (
        !conflict.end_requested ||
        voice.conflictThread !== thread ||
        voice.snapshot ||
        voice.error ||
        voice.recovery ||
        endRequested
      ) {
        restartConsidered.current = true;
      } else if (focused && isConnected && selectionReady) {
        restartConsidered.current = true;
        queueRestart(conflict.id, thread);
      }
    }
  }, [conflict, voice.conflictThread, selectionReady, focused, isConnected]);
  useEffect(() => {
    const intent = restartIntent.current;
    if (!intent) return;
    const replaced =
      conflict &&
      (conflict.id !== intent.sessionId ||
        voice.conflictThread !== intent.thread);
    const endedForAnotherReason =
      sessionTerminal &&
      session?.id === intent.sessionId &&
      session.end_reason !== "user_ended";
    if (
      !focused ||
      !isConnected ||
      intent.scope !== restartScope ||
      endRequested ||
      uncertain ||
      voice.error ||
      voice.recovery ||
      replaced ||
      endedForAnotherReason
    ) {
      if (endedForAnotherReason)
        setRestartNotice(
          "The previous call ended. Start a new call when ready.",
        );
      cancelRestart();
      return;
    }
    if (intent.phase === "starting") {
      if (voice.connected) cancelRestart();
      return;
    }
    if (confirmedEnd && canStart && !closed.current) {
      const starting = { ...intent, phase: "starting" as const };
      restartIntent.current = starting;
      setRestart(starting);
      void start().then((accepted) => {
        if (accepted === false && restartIntent.current === starting) {
          cancelRestart();
          setRestartNotice("The new call did not start. Try again when ready.");
        }
      });
    }
  }, [
    restart,
    restartScope,
    focused,
    isConnected,
    endRequested,
    uncertain,
    voice.error,
    voice.recovery,
    conflict,
    voice.conflictThread,
    session,
    confirmedEnd,
    canStart,
    voice.connected,
    voice.starting,
  ]);
  useEffect(() => {
    if (!restart || restart.phase !== "disconnecting") return;
    // The ordinary six-check budget may finish before the provider closes.
    // An explicit restart owns additional read-only checks only until this deadline.
    const interval = setInterval(() => {
      if (
        restartIntent.current === restart &&
        AppState.currentState === "active" &&
        !endBusy.current
      )
        void voice.checkConflict(false);
    }, 500);
    const timer = setTimeout(() => {
      if (restartIntent.current !== restart) return;
      cancelRestart();
      setRestartExpired(true);
    }, 30_000);
    return () => {
      clearTimeout(timer);
      clearInterval(interval);
    };
  }, [restart]);
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (state) => {
      if (
        state === "background" ||
        (state !== "active" && restartIntent.current?.phase === "disconnecting")
      )
        cancelRestart();
    });
    return () => {
      subscription.remove();
      restartIntent.current = undefined;
    };
  }, []);
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
      (endRequested ||
        (confirmedEnd &&
          !restartIntent.current &&
          !restarting &&
          !restartNotice &&
          !voice.error &&
          !voice.recovery)) &&
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
    voice.error,
    voice.recovery,
    restarting,
    restartNotice,
  ]);
  useEffect(() => {
    if (!preferencesVisible || !focused) setVoicePickerScope(undefined);
  }, [preferencesVisible, focused]);

  function closeOnce() {
    if (!mounted.current || closed.current) return;
    closed.current = true;
    cancelRestart();
    onClose();
  }
  function cancelRestart() {
    restartIntent.current = undefined;
    setRestart(undefined);
  }
  function queueRestart(sessionId: string, previousThread: string) {
    const intent: Restart = {
      scope: restartScope,
      sessionId,
      thread: previousThread,
      phase: "disconnecting",
    };
    restartIntent.current = intent;
    setRestart(intent);
    setRestartExpired(false);
    setRestartNotice(undefined);
    entry.current.attempted = true;
  }
  async function replaceCall() {
    if (
      !conflict ||
      endBusy.current ||
      checkBusy.current ||
      !restartContext.current.allowed ||
      restartContext.current.scope !== restartScope
    )
      return;
    queueRestart(conflict.id, voice.conflictThread ?? thread);
    endBusy.current = true;
    setEndingLocal(true);
    try {
      await voice.endConflict();
    } catch {
      cancelRestart();
    } finally {
      endBusy.current = false;
      if (mounted.current) setEndingLocal(false);
    }
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
    if (!canStart || !option || !chosen) return false;
    entry.current.attempted = true;
    setCallSelection({ option, preferences: chosen });
    setBillingOpenError(false);
    setRestartNotice(undefined);
    return voice.start(chosen, option.voice.protocol, saved, option);
  }
  /** Closing voice ends the call; accepted agent tasks keep running. */
  async function requestEnd() {
    cancelRestart();
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
      if (confirmed === true && !restartIntent.current) closeOnce();
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
    onReview?.(acknowledgementId);
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
            {task.state === "awaiting_confirmation" && onReview ? (
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
  function closePreferences() {
    pickerContext.current.allowed = false;
    setVoicePickerScope(undefined);
    setLocalPreferencesOpen(false);
    onPreferencesClose?.();
  }
  const quiet = { color: colors.textTertiary };
  const controlsBlocked =
    voice.starting ||
    confirmingEnd ||
    !!conflict ||
    uncertain ||
    checking ||
    ending;
  const returnToChat = confirmingEnd || uncertain || checking;
  const endDisabled = ending;
  const showRecovery =
    !ending && (confirmingEnd || !!conflict || uncertain || checking);
  const showPreCall = !active && !showRecovery && !ending && !confirmingEnd;
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
            {voice.starting || checking || ending || restarting ? (
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
              returnToChat
                ? "Return to chat"
                : conflict
                  ? "End existing call"
                  : hasCall
                    ? "End voice"
                    : "Return to chat"
            }
            accessibilityState={{ disabled: endDisabled }}
            disabled={endDisabled}
            onPress={() => (returnToChat ? returnToText() : void requestEnd())}
            style={[
              styles.end,
              {
                backgroundColor: colors.textPrimary,
                opacity: endDisabled ? 0.45 : 1,
              },
            ]}
          >
            {returnToChat ? (
              <X size={17} color={colors.bg} />
            ) : (
              <PhoneOff size={17} color={colors.bg} />
            )}
            <Text style={[styles.controlText, { color: colors.bg }]}>
              {returnToChat ? "Back to chat" : hasCall ? "End" : "Close"}
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
        {showRecovery ? (
          <View style={styles.recovery}>
            <Label muted>
              {restarting
                ? "Disconnecting the previous call. Your new call will start once disconnection is confirmed."
                : restartExpired
                  ? "The previous call is still disconnecting. Check its status and try again shortly."
                  : checking
                    ? "Checking whether the previous call has ended. This does not reconnect your microphone."
                    : confirmingEnd
                      ? "The previous call is finishing. A new call can start once it closes. You can continue with text."
                      : conflict
                        ? voice.conflictThread &&
                          voice.conflictThread !== thread
                          ? "A voice call is open in another chat. Only one voice call can run at a time. End that call to start a new one here, or continue with text."
                          : "A voice call is already open. Only one voice call can run at a time. End it to start a new one, or continue with text."
                        : "The previous call's status has not been confirmed. Check whether it has ended before starting a new call, or continue with text."}
            </Label>
            <View style={styles.recoveryActions}>
              {conflict && !restarting ? (
                <ActionButton
                  label="End previous call and start here"
                  compact
                  disabled={!canReplaceCall}
                  onPress={() => void replaceCall()}
                />
              ) : null}
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
            <Notice message={voice.recovery.message} tone="error" />
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
                showErrorDetails ? "Hide error details" : "Show error details"
              }
              accessibilityState={{
                expanded: showErrorDetails,
              }}
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
        {restartNotice ? <Notice message={restartNotice} /> : null}
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
        {showPreCall ? (
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
                  message={t(
                    options.error instanceof Error
                      ? options.error.message
                      : settings.error instanceof Error
                        ? settings.error.message
                        : "Voice options could not load.",
                  )}
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
        {needsRetry && showPreCall ? (
          <ActionButton
            label="Try again"
            variant="secondary"
            compact
            disabled={!canStart}
            onPress={() => void start()}
          />
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
                    voicePickerContent.current = {
                      options: option.voice.voices.map((row) => ({
                        value: row.id,
                        label: row.label,
                      })),
                      value: nextVoice ? [nextVoice] : [],
                    };
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
      {preferencesVisible && focused && voicePickerScope === selectionScope ? (
        <WorkspaceSelectionSheet
          key={selectionScope}
          scopeKey={selectionScope}
          label="Voice"
          title="Choose voice"
          presentationStyle="overFullScreen"
          keepOpenWhenDisabled
          description="Changes apply to your next call."
          options={voicePickerContent.current.options}
          value={voicePickerContent.current.value}
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
