import { useReducedMotion } from "../../../lib/accessibility/useReducedMotion";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
  Animated,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import {
  Ellipsis,
  Mic,
  MicOff,
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
import {
  ScrollEdgeFades,
  useScrollEdges,
} from "../../../components/workspace/ScrollEdgeFades";
import { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { useNetworkStatus } from "../../../hooks/useNetworkStatus";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { ChatComposerFrame } from "../ChatComposer";
import { useNativeVoice } from "./useNativeVoice";
import { useVoiceLevels } from "./useVoiceLevels";
import { VoiceComposerAura } from "./VoiceComposerAura";
import { VoiceMediaBridge } from "./VoiceMediaBridge";
import { voiceAuraPhase, voiceAuraStatus } from "./voiceAuraState";
import { chooseVoiceOption, voiceElapsed, voiceTariff } from "./voiceSelection";
import type { VoicePreferences, VoiceSnapshot } from "./voiceContracts";

type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  onUpdate: () => void;
  /** Scrolls the chat above to a card the call is waiting on. */
  onReview: (acknowledgementId?: string) => void;
};
type Task = VoiceSnapshot["tasks"][number];

/**
 * The voice composer keeps one size whether a call is idle or live and in
 * either mode: the middle area and the hint line are always reserved.
 */
const BODY_HEIGHT = 136;

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
    <VoiceConversation {...props} environment={setup.environment} />
  ) : (
    <ChatComposerFrame style={styles.frame} bounce={false}>
      <Notice message={setup.error ?? "Voice is unavailable."} tone="error" />
      <ActionButton label="Return to chat" variant="secondary" onPress={props.onClose} />
    </ChatComposerFrame>
  );
}

// The same plain words as the web voice panel.
const TASK_STATE: Record<Task["state"], string> = {
  queued: "Queued",
  claimed: "Running",
  awaiting_confirmation: "Needs your OK",
  completed: "Done",
  cancelled: "Cancelled",
};

function VoiceConversation({
  thread,
  agentName,
  onClose,
  onUpdate,
  onReview,
  environment,
}: Props & {
  environment: ReturnType<typeof nativeVoiceEnvironment>;
}) {
  const { colors, mode } = useTheme();
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const levels = useVoiceLevels(voice.media);
  const [hold] = useState(() => new Animated.Value(0));
  const [holdStartedAt, setHoldStartedAt] = useState<number>();
  const [rise] = useState(() => new Animated.Value(reducedMotion ? 0 : 1));
  const transcriptEdges = useScrollEdges();
  const [tasksOpen, setTasksOpen] = useState(false);
  const [moreOpen, setMoreOpen] = useState(false);
  const [pricingOpen, setPricingOpen] = useState(false);
  const [showErrorDetails, setShowErrorDetails] = useState(false);
  const [billingOpenError, setBillingOpenError] = useState(false);
  const [now, setNow] = useState(Date.now());
  const transcript = useRef<ScrollView>(null);
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
  const option = chooseVoiceOption(choices);
  const isGrok = option?.voice.protocol === "xai_realtime";
  const [modeChoice, setModeChoice] =
    useState<VoicePreferences["input_mode"]>();
  const inputMode = isGrok
    ? "push_to_talk"
    : (modeChoice ?? saved?.input_mode ?? "push_to_talk");
  const voiceId =
    option?.voice.voices.find((row) => row.id === saved?.voice)?.id ??
    option?.voice.voices[0]?.id ??
    null;
  const voiceLabel = option?.voice.voices.find(
    (row) => row.id === voiceId,
  )?.label;
  const active = voice.starting || voice.connected;
  const session = voice.snapshot?.session;
  const muted = session?.muted ?? false;
  const microphoneBlocked =
    muted ||
    !!session?.input_muted ||
    !!session?.end_requested ||
    session?.state !== "active";
  const captions = voice.snapshot?.captions ?? [];
  const tasks = voice.snapshot?.tasks ?? [];
  const phase = voiceAuraPhase({
    starting: voice.starting,
    connected: voice.connected,
    inputMode,
    holding: voice.holding,
    microphoneBlocked,
    muted,
    outputActive: levels.speaking,
    captions,
    tasks,
  });
  const status = voice.starting
    ? "Connecting…"
    : voice.connected
      ? session?.state === "closing"
        ? "Ending…"
        : voiceAuraStatus(phase, inputMode)
      : voice.recovery || (voice.error && !voice.snapshot)
        ? "Unable to start"
        : voice.conflict
          ? "Call already active"
          : voice.snapshot
            ? "Call ended"
            : "Ready to talk";
  const duration = voiceElapsed(session?.created_at, session?.closed_at, now);
  const talking = holdStartedAt
    ? voiceElapsed(new Date(holdStartedAt).toISOString(), null, now)
    : undefined;
  const canStart =
    !!option?.available &&
    !active &&
    !voice.conflict &&
    isConnected &&
    options.isSuccess &&
    settings.isSuccess;
  const running = tasks.some(
    (task) => task.state === "queued" || task.state === "claimed",
  );
  const needsOk = tasks.some((task) => task.state === "awaiting_confirmation");
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  useEffect(() => {
    if (!focused) onClose();
  }, [focused, onClose]);
  useEffect(() => {
    if (!isConnected) void voice.end();
  }, [isConnected]);
  useEffect(() => {
    setShowErrorDetails(false);
  }, [voice.error]);

  useEffect(() => {
    setHoldStartedAt(voice.holding ? Date.now() : undefined);
    if (voice.holding) setNow(Date.now());
  }, [voice.holding]);
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

  /** Closing voice ends the call; accepted agent tasks keep running. */
  function close() {
    if (voice.holding) voice.hold(false);
    void voice.end();
    onClose();
  }
  /** The chat stays on screen above the call, so a card is shown in place. */
  function review(acknowledgementId?: string) {
    if (voice.holding) voice.hold(false);
    onReview(acknowledgementId);
  }
  function preferences(
    mode: VoicePreferences["input_mode"],
  ): VoicePreferences | undefined {
    if (!option) return undefined;
    return {
      service_id: option.service_id,
      connection_id: option.connection_id,
      key_source: option.key_source,
      model: option.model,
      voice: voiceId,
      input_mode: mode,
      language: saved?.language ?? null,
      notify_on_completion: saved?.notify_on_completion ?? false,
    };
  }
  async function start() {
    const chosen = preferences(inputMode);
    if (!canStart || !option || !chosen) return;
    setBillingOpenError(false);
    await voice.start(chosen, option.voice.protocol);
  }
  /** Hands-free or Hold to talk, from the call itself; saved for next time. */
  function changeMode(mode: VoicePreferences["input_mode"]) {
    if (isGrok || mode === inputMode) return;
    if (active && !voice.setInputMode(mode)) return;
    setModeChoice(mode);
    const chosen = preferences(mode);
    if (chosen)
      void environment.api
        .savePreferences(chosen)
        .then(() => settings.refetch())
        .catch(() => {});
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
  function taskRow(task: Task) {
    return (
      <View key={task.id} style={styles.taskRow}>
        <View style={styles.fill}>
          <Label numberOfLines={1}>{task.title || "Agent task"}</Label>
          <Label muted style={styles.taskState} numberOfLines={2}>
            {TASK_STATE[task.state]}
            {task.progress?.detail ? ` · ${task.progress.detail}` : ""}
          </Label>
        </View>
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
    );
  }

  const quiet = { color: colors.textTertiary };
  const surface = colors.segmentTrack;
  const main = colors.textPrimary;
  const onMain = colors.bg;
  const tray = tasksOpen || moreOpen;
  return (
    <>
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
          opacity: rise.interpolate({ inputRange: [0, 1], outputRange: [1, 0] }),
        }}
      >
        {/* The aura around the composer says it is in voice mode and moves
            with NyxBot's voice and yours. */}
        <VoiceComposerAura
          phase={phase}
          input={levels.input}
          output={levels.output}
          reducedMotion={reducedMotion}
        >
        <ChatComposerFrame style={styles.frame} bounce={false}>
          <View style={styles.header}>
            <View style={styles.headerText}>
              <Text numberOfLines={1} style={[styles.title, { color: colors.textPrimary }]}>
                {agentName}
              </Text>
              <Text numberOfLines={1} style={[styles.status, { color: voice.error && !voice.connected ? colors.danger : colors.textSecondary }]}>
                <Text accessibilityLiveRegion="polite">{status}</Text>
                {active && inputMode === "automatic" ? (
                  <Text accessibilityLabel={`Call time ${duration}`} style={[styles.elapsed, quiet]}>
                    {`  ${duration}`}
                  </Text>
                ) : active && talking ? (
                  <Text accessibilityLabel={`Talking for ${talking}`} style={[styles.elapsed, quiet]}>
                    {`  ${talking}`}
                  </Text>
                ) : null}
              </Text>
            </View>
            {tasks.length ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={tasksOpen ? "Hide agent tasks" : "Show agent tasks"}
                accessibilityState={{ expanded: tasksOpen }}
                onPress={() => {
                  setMoreOpen(false);
                  setTasksOpen((value) => !value);
                }}
                style={[
                  styles.chip,
                  { backgroundColor: needsOk ? colors.warningTone.bg : surface },
                ]}
              >
                {running ? (
                  <ActivityIndicator size="small" color={colors.textSecondary} />
                ) : null}
                <Text style={[styles.chipText, { color: needsOk ? colors.warningTone.text : colors.textPrimary }]}>
                  {needsOk
                    ? "Needs your OK"
                    : `${tasks.length} ${tasks.length === 1 ? "task" : "tasks"}`}
                </Text>
              </Pressable>
            ) : null}
            {active ? (
              <>
                <IconButton
                  label={voice.speakerMuted ? "Enable speaker" : "Mute speaker"}
                  selected={voice.speakerMuted}
                  disabled={voice.starting}
                  onPress={() => voice.muteSpeaker(!voice.speakerMuted)}
                >
                  {voice.speakerMuted ? (
                    <VolumeX size={18} color={colors.textPrimary} />
                  ) : (
                    <Volume2 size={18} color={colors.textPrimary} />
                  )}
                </IconButton>
                <IconButton
                  label={moreOpen ? "Hide call options" : "Call options"}
                  selected={moreOpen}
                  onPress={() => {
                    setTasksOpen(false);
                    setMoreOpen((value) => !value);
                  }}
                >
                  <Ellipsis size={18} color={colors.textPrimary} />
                </IconButton>
              </>
            ) : null}
          </View>
          {/* Task list and call options open inside the composer, never over the chat. */}
          <View style={styles.body}>
          {tray ? (
            <ScrollView
              style={[styles.tray, { backgroundColor: surface }]}
              contentContainerStyle={styles.trayContent}
            >
              {tasksOpen ? (
                <>
                  <Label style={styles.trayTitle}>{`Agent tasks · ${tasks.length}`}</Label>
                  {tasks.slice(-5).map(taskRow)}
                  <Text style={[styles.footnote, quiet]}>
                    Ending the call leaves accepted tasks running. Stop a task
                    here to cancel it.
                  </Text>
                </>
              ) : (
                <>
                  <Text style={[styles.footnote, { color: colors.textSecondary }]}>
                    {inputMode === "automatic"
                      ? "Your microphone is on while unmuted."
                      : "Audio is sent only while you hold the microphone."}{" "}
                    Use headphones if you hear echo.
                    {option && (option.pricing || option.key_source === "own")
                      ? ` ${voiceTariff(option)}`
                      : ""}
                  </Text>
                  <ActionButton
                    label="No sound? Resume audio"
                    variant="secondary"
                    compact
                    disabled={voice.starting}
                    onPress={() => {
                      setMoreOpen(false);
                      void voice.resumeAudio();
                    }}
                  />
                </>
              )}
            </ScrollView>
          ) : (
          <View style={styles.transcriptFrame}>
            <ScrollView
              ref={transcript}
              style={styles.fill}
              contentContainerStyle={[
                styles.transcriptContent,
                !active && !captions.length && styles.idleContent,
              ]}
              keyboardShouldPersistTaps="handled"
              scrollEventThrottle={32}
              onLayout={(event) =>
                transcriptEdges.layout(event.nativeEvent.layout.height)
              }
              onScroll={(event) =>
                transcriptEdges.scroll(event.nativeEvent.contentOffset.y)
              }
              // The newest words stay in view; scroll up for the rest.
              onContentSizeChange={(_width, height) => {
                transcriptEdges.content(height);
                transcript.current?.scrollToEnd?.({ animated: !reducedMotion });
              }}
            >
            {voice.recovery ? (
              <Card>
                <Notice message={voice.recovery.message} tone="error" />
                <ActionButton
                  label="Review billing on Web"
                  variant="secondary"
                  onPress={openBilling}
                />
                <ActionButton
                  label="Continue with text"
                  variant="secondary"
                  onPress={close}
                />
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
                    {showErrorDetails
                      ? "Hide error details"
                      : "Show error details"}
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
            {voice.conflict ? (
              <Card>
                <Label>
                  A voice call is already open. End it before starting another
                  call here.
                </Label>
                <ActionButton
                  label="End existing call"
                  variant="danger"
                  loading={voice.endingConflict}
                  onPress={() => void voice.endConflict()}
                />
              </Card>
            ) : null}
            {session?.idle_warning ? (
              <Notice
                message={
                  inputMode === "automatic"
                    ? "The call will end soon because you haven’t spoken. Say something to continue."
                    : "The call will end soon because you haven’t spoken. Hold to talk to continue."
                }
              />
            ) : null}
            {!active ? (
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
                {options.isSuccess && !choices.length ? (
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
              </>
            ) : null}
            {!active && !captions.length ? (
              <View style={styles.idle}>
                <Text style={[styles.idleTitle, { color: colors.textPrimary }]}>
                  Hey, what can I do for you today?
                </Text>
                {option ? (
                  <Text style={[styles.footnote, styles.center, quiet]}>
                    {voiceLabel ? `${voiceLabel} voice · ` : ""}
                    {isGrok ? "Hold to talk · " : ""}
                    Change in Settings › Voice{"\n"}
                    {voiceTariff(option)}
                  </Text>
                ) : null}
              </View>
            ) : null}
            {captions.map((caption, index) => {
                const user = caption.speaker === "user";
                // Like chat: your words in a bubble, the agent's as plain text.
                return user ? (
                  <View key={caption.id} style={[styles.userBubble, { backgroundColor: surface }]}>
                    <Text selectable style={[styles.bubbleText, { color: caption.complete ? colors.textPrimary : colors.textSecondary }]}>
                      {caption.text}
                    </Text>
                  </View>
                ) : (
                  <Text
                    key={caption.id}
                    selectable
                    style={[
                      index === captions.length - 1 ? styles.agentText : styles.agentEarlier,
                      { color: caption.complete ? colors.textPrimary : colors.textSecondary },
                    ]}
                  >
                    {caption.text}
                  </Text>
                );
              })}
              {tasks
                .filter((task) => task.state === "queued" || task.state === "claimed")
                .map((task) => (
                  <View key={`work:${task.id}`} style={styles.workLine}>
                    <ActivityIndicator size="small" color={colors.textTertiary} />
                    <Text numberOfLines={2} style={[styles.workText, quiet]}>
                      {TASK_STATE[task.state]}
                      {" · "}
                      {task.progress?.detail || task.title || "Agent task"}
                    </Text>
                  </View>
                ))}
            </ScrollView>
            <ScrollEdgeFades edges={transcriptEdges.edges} color={colors.composerBg} />
          </View>
          )}
          </View>
          {/* Grok runs on explicit holds only, so it has no mode to choose. */}
          {option && !isGrok ? (
            <View
              accessibilityRole="radiogroup"
              accessibilityLabel="Listening mode"
              style={[styles.modes, { backgroundColor: surface }]}
            >
              {(["automatic", "push_to_talk"] as const).map((mode) => {
                const selected = inputMode === mode;
                return (
                  <Pressable
                    key={mode}
                    accessibilityRole="radio"
                    accessibilityLabel={
                      mode === "automatic" ? "Hands-free mode" : "Hold to talk mode"
                    }
                    accessibilityHint={
                      mode === "automatic"
                        ? "NyxBot listens and answers when you pause"
                        : "NyxBot listens while you hold the microphone"
                    }
                    accessibilityState={{
                      selected,
                      checked: selected,
                      disabled: voice.starting,
                    }}
                    disabled={voice.starting}
                    onPress={() => changeMode(mode)}
                    style={[
                      styles.mode,
                      selected && { backgroundColor: colors.segmentSelected },
                    ]}
                  >
                    <Text
                      style={[
                        styles.modeLabel,
                        { color: selected ? colors.textPrimary : colors.textSecondary },
                      ]}
                    >
                      {mode === "automatic" ? "Hands-free" : "Hold to talk"}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
          ) : (
            <View style={styles.modesPlaceholder}>
              {isGrok ? (
                <Text style={[styles.modeLabel, { color: colors.textSecondary }]}>
                  Hold to talk
                </Text>
              ) : null}
            </View>
          )}
          <View style={styles.controls}>
            {active && inputMode === "push_to_talk" ? (
              <IconButton
                large
                label={muted ? "Unmute microphone" : "Mute microphone"}
                selected={muted}
                disabled={voice.starting}
                onPress={() => void voice.mute(!muted)}
              >
                {/* Always the mute symbol, so it never reads as a second mic. */}
                <MicOff size={20} color={colors.textPrimary} />
              </IconButton>
            ) : (
              <View style={styles.sideSpacer} />
            )}
            {!active ? (
              <View style={styles.holdColumn}>
                <MainButton
                  label={voice.error ? "Try again" : "Start voice"}
                  fill={main}
                  disabled={!canStart}
                  onPress={() => void start()}
                >
                  <Mic size={26} color={onMain} />
                </MainButton>
                <Text style={[styles.holdHint, { color: colors.textSecondary }]}>
                  {voice.error ? "Try again" : "Tap to start"}
                </Text>
              </View>
            ) : inputMode === "push_to_talk" ? (
              <View style={styles.holdColumn}>
                <View style={styles.holdStage}>
                  <Animated.View
                    pointerEvents="none"
                    style={[
                      styles.halo,
                      {
                        backgroundColor: main,
                        opacity: hold.interpolate({ inputRange: [0, 1], outputRange: [0, 0.22] }),
                        transform: [{ scale: hold.interpolate({ inputRange: [0, 1], outputRange: [1, 1.5] }) }],
                      },
                    ]}
                  />
                  <Animated.View
                    style={{
                      transform: [{ scale: hold.interpolate({ inputRange: [0, 1], outputRange: [1, 1.1] }) }],
                    }}
                  >
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel="Hold to talk"
                      accessibilityHint={voice.holding ? "Release to send" : undefined}
                      accessibilityState={{
                        disabled: microphoneBlocked || !voice.connected,
                        selected: voice.holding,
                      }}
                      disabled={microphoneBlocked || !voice.connected}
                      onPressIn={() => voice.hold(true)}
                      onPressOut={() => voice.hold(false)}
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
                        styles.main,
                        {
                          backgroundColor: main,
                          opacity: microphoneBlocked || !voice.connected ? 0.4 : 1,
                        },
                      ]}
                    >
                      <Mic size={26} color={onMain} />
                    </Pressable>
                  </Animated.View>
                </View>
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
                  disabled={voice.starting}
                  onPress={() => void voice.mute(!muted)}
                >
                  {muted ? (
                    <MicOff size={26} color={colors.onPrimary} />
                  ) : (
                    <Mic size={26} color={onMain} />
                  )}
                </MainButton>
                {/* The same line as Hold to talk's instruction keeps one size. */}
                <Text
                  accessibilityElementsHidden
                  importantForAccessibility="no"
                  style={[styles.holdHint, { color: colors.textSecondary }]}
                >
                  {muted ? "Muted" : ""}
                </Text>
              </View>
            )}
            <IconButton
              large
              label={active ? "End voice and close" : "Close voice"}
              onPress={close}
            >
              {active && inputMode === "automatic" ? (
                <PhoneOff size={20} color={colors.danger} />
              ) : (
                <X size={20} color={active ? colors.danger : colors.textPrimary} />
              )}
            </IconButton>
          </View>
        </ChatComposerFrame>
        </VoiceComposerAura>
      </Animated.View>
    </>
  );
}

/** A round control inside the voice composer; selected toggles fill. */
function IconButton({
  label,
  selected,
  disabled = false,
  large = false,
  onPress,
  children,
}: {
  label: string;
  selected?: boolean;
  disabled?: boolean;
  large?: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled, selected: !!selected }}
      disabled={disabled}
      hitSlop={large ? 4 : 6}
      onPress={onPress}
      style={({ pressed }) => [
        large ? styles.iconLarge : styles.icon,
        {
          backgroundColor: selected ? colors.segmentSelected : colors.segmentTrack,
          opacity: disabled ? 0.4 : pressed ? 0.7 : 1,
        },
      ]}
    >
      {children}
    </Pressable>
  );
}

/** The centre action: Start, then the microphone. */
function MainButton({
  label,
  fill,
  disabled,
  onPress,
  children,
}: {
  label: string;
  fill: string;
  disabled: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        styles.main,
        { backgroundColor: fill, opacity: disabled ? 0.4 : pressed ? 0.8 : 1 },
      ]}
    >
      {children}
    </Pressable>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  center: { textAlign: "center" },
  frame: { gap: 10, paddingHorizontal: 12, paddingTop: 12, paddingBottom: 10 },
  header: { flexDirection: "row", alignItems: "center", gap: 10, minHeight: 36, paddingLeft: 4 },
  body: { height: BODY_HEIGHT },
  idle: { alignItems: "center", gap: 8 },
  idleContent: { flexGrow: 1, justifyContent: "center" },
  idleTitle: { fontFamily: fonts.displaySemi, fontSize: 18, lineHeight: 24, textAlign: "center" },
  trayContent: { padding: 12, gap: 10 },
  modesPlaceholder: { minHeight: 38, alignItems: "center", justifyContent: "center" },
  headerText: { flex: 1, minWidth: 0 },
  title: { fontFamily: fonts.bodySemi, fontSize: 15, lineHeight: 20 },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  chip: {
    flexDirection: "row",
    alignItems: "center",
    gap: 6,
    minHeight: 32,
    borderRadius: 16,
    paddingHorizontal: 10,
  },
  chipText: { fontFamily: fonts.bodySemi, fontSize: 12, lineHeight: 16 },
  icon: { width: 36, height: 36, borderRadius: 18, alignItems: "center", justifyContent: "center" },
  iconLarge: { width: 48, height: 48, borderRadius: 24, alignItems: "center", justifyContent: "center" },
  tray: { flex: 1, borderRadius: 14 },
  trayTitle: { fontFamily: fonts.bodySemi, fontSize: 14 },
  taskRow: { flexDirection: "row", alignItems: "center", gap: 10 },
  taskState: { fontSize: 12 },
  transcriptFrame: { flex: 1, position: "relative" },
  transcriptContent: { gap: 10, paddingHorizontal: 2, paddingBottom: 2 },
  userBubble: {
    alignSelf: "flex-end",
    maxWidth: "85%",
    borderRadius: 18,
    borderBottomRightRadius: 6,
    paddingHorizontal: 12,
    paddingVertical: 8,
  },
  bubbleText: { fontFamily: fonts.bodyRegular, fontSize: 15, lineHeight: 21 },
  agentText: { fontFamily: fonts.bodyRegular, fontSize: 16, lineHeight: 23 },
  agentEarlier: { fontFamily: fonts.bodyRegular, fontSize: 14, lineHeight: 20 },
  workLine: { flexDirection: "row", alignItems: "center", gap: 8 },
  workText: { flex: 1, fontFamily: fonts.body, fontSize: 12, lineHeight: 17 },
  textButton: { minHeight: 32, justifyContent: "center", alignSelf: "center" },
  footnote: { fontFamily: fonts.body, fontSize: 12, lineHeight: 17 },
  modes: { flexDirection: "row", alignSelf: "center", borderRadius: 18, padding: 3 },
  mode: {
    minHeight: 32,
    minWidth: 108,
    paddingHorizontal: 12,
    borderRadius: 15,
    alignItems: "center",
    justifyContent: "center",
  },
  modeLabel: { fontFamily: fonts.bodySemi, fontSize: 13, lineHeight: 18 },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 8,
  },
  sideSpacer: { width: 48, height: 48 },
  main: {
    width: 64,
    height: 64,
    borderRadius: 32,
    alignItems: "center",
    justifyContent: "center",
  },
  holdColumn: { alignItems: "center", gap: 6 },
  holdStage: { width: 64, height: 64, alignItems: "center", justifyContent: "center" },
  halo: { position: "absolute", width: 64, height: 64, borderRadius: 32 },
  holdHint: { fontFamily: fonts.bodySemi, fontSize: 12, lineHeight: 16, minHeight: 16 },
});
