import { useNativeSheetDismiss } from "../../../components/workspace/useNativeSheetDismiss";
import { useReducedMotion } from "../../../lib/accessibility/useReducedMotion";
import { useEffect, useRef, useState, type ReactNode } from "react";
import {
  ActivityIndicator,
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
import { LinearGradient } from "expo-linear-gradient";
import {
  ChevronDown,
  ChevronLeft,
  ChevronUp,
  MessageSquareText,
  Mic,
  MicOff,
  PhoneOff,
  Volume2,
  VolumeX,
  X,
} from "lucide-react-native";
import { WorkspaceSheetSurface } from "../../../components/workspace/WorkspaceSheetSurface";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../../components/workspace/WorkspaceUI";
import { nativeVoiceEnvironment } from "../../../lib/api/assistantVoiceApi";
import { useNetworkStatus } from "../../../hooks/useNetworkStatus";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts } from "../../../theme/designTokens";
import { useNativeVoice } from "./useNativeVoice";
import { useVoiceLevels } from "./useVoiceLevels";
import { VoiceAura } from "./VoiceAura";
import { VoiceMediaBridge } from "./VoiceMediaBridge";
import { voiceAuraPhase, voiceAuraStatus } from "./voiceAuraState";
import { chooseVoiceOption, voiceElapsed, voiceTariff } from "./voiceSelection";
import type { VoicePreferences, VoiceSnapshot } from "./voiceContracts";

type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  onUpdate: () => void;
  onReview: (acknowledgementId?: string) => void;
  /** The chat's pending action cards and approvals, decided here in place. */
  actions?: ReactNode;
  actionCount?: number;
};
type Task = VoiceSnapshot["tasks"][number];

export function AssistantVoiceConversation(props: Props) {
  const sheet = useNativeSheetDismiss(props.onClose);
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
    <Modal
      {...sheet.modalProps}
      transparent
      onRequestClose={() => sheet.close()}
    >
      <WorkspaceSheetSurface nativePresentation onClose={() => sheet.close()}>
        <View style={styles.errorBody}>
          <Notice
            message={setup.error ?? "Voice is unavailable."}
            tone="error"
          />
          <ActionButton label="Return to chat" onPress={() => sheet.close()} />
        </View>
      </WorkspaceSheetSurface>
    </Modal>
  );
}

/** The task a collapsed tray shows: what needs you first, then live work. */
function headlineTask(tasks: readonly Task[]) {
  return (
    tasks.find((task) => task.state === "awaiting_confirmation") ??
    [...tasks]
      .reverse()
      .find((task) => task.state === "queued" || task.state === "claimed") ??
    tasks[tasks.length - 1]
  );
}

/**
 * Full-screen voice mode: agent and status on top, the aura in the middle,
 * live captions under it, and round call controls at the bottom. Voice,
 * conversation mode and completion notices are chosen in Settings › Voice.
 */
function VoiceConversation({
  thread,
  agentName,
  onClose,
  onUpdate,
  onReview,
  actions,
  actionCount = 0,
  environment,
}: Props & {
  environment: ReturnType<typeof nativeVoiceEnvironment>;
}) {
  const { colors, mode } = useTheme();
  const insets = useSafeAreaInsets();
  const dimensions = useWindowDimensions();
  const sheet = useNativeSheetDismiss(onClose);
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const levels = useVoiceLevels(voice.media);
  const [minimized, setMinimized] = useState(false);
  const [tasksOpen, setTasksOpen] = useState(false);
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
  const canStart =
    !!option?.available &&
    !active &&
    !voice.conflict &&
    isConnected &&
    options.isSuccess &&
    settings.isSuccess;
  const auraSize = Math.round(
    Math.max(160, Math.min(dimensions.width * 0.62, dimensions.height * 0.3, 280)),
  );
  const headline = headlineTask(tasks);
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

  function close() {
    void voice.end();
    if (minimized) onClose();
    else sheet.close();
  }
  function minimize() {
    voice.hold(false);
    setMinimized(true);
  }
  function review(acknowledgementId?: string) {
    minimize();
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
          <Label muted style={styles.taskState}>
            {task.state.replaceAll("_", " ")}
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
  const dark = mode === "dark";
  // Floating controls: white on the light wash, frosted on the dark one; the
  // centre action is the page's one solid, dark (or light) disc.
  const surface = dark ? "rgba(255,255,255,0.14)" : "#FFFFFF";
  const main = dark ? "#F4F2FA" : "#141218";
  const onMain = dark ? "#141218" : "#FFFFFF";
  return (
    <>
      {voice.media ? (
        <VoiceMediaBridge
          key={thread + ":" + voice.mediaAttempt}
          media={voice.media}
          origin={environment.origin}
        />
      ) : null}
      {minimized ? (
        <View
          style={[
            styles.minimized,
            {
              backgroundColor: colors.primaryTone.bg,
              borderColor: colors.primaryTone.border,
            },
          ]}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Restore voice call"
            onPress={() => setMinimized(false)}
            style={styles.restore}
          >
            <Mic size={20} color={colors.primaryOnTint} />
            <Label style={styles.fill} numberOfLines={1}>
              {status} · {agentName}
              {active ? ` · ${duration}` : ""}
            </Label>
            <ChevronUp size={18} color={colors.textSecondary} />
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="End voice and close"
            onPress={close}
            style={styles.iconButton}
          >
            <PhoneOff size={20} color={colors.danger} />
          </Pressable>
        </View>
      ) : null}
      {/* Opaque overFullScreen rather than fullScreen: a fullScreen
          presentation detaches the chat screen underneath, including the
          hidden media WebView, whose page would then report itself hidden
          and end the call. */}
      <Modal
        {...sheet.modalProps}
        visible={!minimized && sheet.modalProps.visible}
        transparent
        animationType={reducedMotion ? "none" : "slide"}
        presentationStyle="overFullScreen"
        onRequestClose={close}
      >
        <View
          accessibilityViewIsModal
          style={[
            styles.screen,
            {
              paddingTop: insets.top + 4,
              paddingBottom: Math.max(insets.bottom, 16),
            },
          ]}
        >
          {/* A soft pastel wash, as in a voice assistant's own page. */}
          <LinearGradient
            pointerEvents="none"
            colors={dark ? DARK_WASH : LIGHT_WASH}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 1 }}
            style={StyleSheet.absoluteFill}
          />
          <View style={styles.topBar}>
            <RoundButton
              label={active ? "Minimize voice call" : "Close voice"}
              surface={surface}
              onPress={active ? minimize : close}
            >
              <ChevronLeft size={20} color={colors.textPrimary} />
            </RoundButton>
            <Text
              accessibilityRole="header"
              numberOfLines={1}
              style={[styles.pageTitle, { color: colors.textPrimary }]}
            >
              Voice
            </Text>
            {active ? (
              <RoundButton
                label={voice.speakerMuted ? "Enable speaker" : "Mute speaker"}
                surface={voice.speakerMuted ? colors.textPrimary : surface}
                selected={voice.speakerMuted}
                disabled={voice.starting}
                onPress={() => voice.muteSpeaker(!voice.speakerMuted)}
              >
                {voice.speakerMuted ? (
                  <VolumeX size={18} color={colors.bg} />
                ) : (
                  <Volume2 size={18} color={colors.textPrimary} />
                )}
              </RoundButton>
            ) : (
              <View style={styles.roundSpacer} />
            )}
          </View>
          <View style={styles.stage}>
            <VoiceAura
              phase={phase}
              input={levels.input}
              output={levels.output}
              reducedMotion={reducedMotion}
              size={auraSize}
            />
          </View>
          <View style={styles.speech}>
            <View style={[styles.pill, { backgroundColor: surface }]}>
              <Text numberOfLines={1} style={[styles.pillText, { color: colors.textSecondary }]}>
                {agentName}
              </Text>
            </View>
            <View style={styles.statusRow}>
              <Text
                accessibilityLiveRegion="polite"
                style={[
                  styles.status,
                  {
                    color:
                      voice.error && !voice.connected
                        ? colors.danger
                        : colors.textSecondary,
                  },
                ]}
              >
                {status}
              </Text>
              {active ? (
                <Text
                  accessibilityLabel={`Call time ${duration}`}
                  style={[styles.elapsed, quiet]}
                >
                  {duration}
                </Text>
              ) : null}
            </View>
            {!active && !captions.length ? (
              <View style={styles.intro}>
                <Text style={[styles.introTitle, { color: colors.textPrimary }]}>
                  Hey, what can I do for you today?
                </Text>
                {option ? (
                  <Text style={[styles.introDetail, quiet]}>
                    {voiceLabel ? `${voiceLabel} voice · ` : ""}
                    {isGrok ? "Hold to talk · " : ""}
                    Change in Settings › Voice
                  </Text>
                ) : null}
              </View>
            ) : null}
          </View>
          <ScrollView
            ref={transcript}
            style={[styles.transcript, { maxHeight: dimensions.height * 0.34 }]}
            contentContainerStyle={styles.transcriptContent}
            keyboardShouldPersistTaps="handled"
            // The newest words stay in view; scroll up for the rest.
            onContentSizeChange={() =>
              transcript.current?.scrollToEnd?.({ animated: !reducedMotion })
            }
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
            {captions.map((caption, index) => {
              const recent = index >= captions.length - 2;
              const user = caption.speaker === "user";
              return (
                <View key={caption.id} style={[styles.caption, recent && styles.captionRecent]}>
                  <Text style={[styles.speaker, quiet]}>
                    {user ? "You" : agentName}
                  </Text>
                  <Text
                    selectable
                    style={[
                      recent ? styles.captionText : styles.captionEarlier,
                      {
                        color: !caption.complete
                          ? colors.textTertiary
                          : recent && !user
                            ? colors.textPrimary
                            : colors.textSecondary,
                      },
                    ]}
                  >
                    {caption.text}
                  </Text>
                </View>
              );
            })}
          </ScrollView>
          {actionCount > 0 ? (
            // What the chat is waiting on, decided without leaving the call.
            <View
              style={[
                styles.actions,
                styles.floating,
                { backgroundColor: surface, maxHeight: dimensions.height * 0.42 },
              ]}
            >
              <Text
                accessibilityRole="header"
                accessibilityLiveRegion="polite"
                style={[styles.trayTitle, styles.actionsTitle, { color: colors.textPrimary }]}
              >
                {`${agentName} needs you · ${actionCount}`}
              </Text>
              <ScrollView
                contentContainerStyle={styles.actionsContent}
                keyboardShouldPersistTaps="handled"
              >
                {actions}
              </ScrollView>
            </View>
          ) : null}
          {tasks.length ? (
            <View style={[styles.tray, styles.floating, { backgroundColor: surface }]}>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={
                  tasksOpen ? "Hide agent tasks" : "Show all agent tasks"
                }
                accessibilityState={{ expanded: tasksOpen }}
                onPress={() => setTasksOpen((value) => !value)}
                style={styles.trayHeader}
              >
                <Label style={[styles.fill, styles.trayTitle]}>
                  {`Agent tasks · ${tasks.length}`}
                </Label>
                {tasksOpen ? (
                  <ChevronDown size={18} color={colors.textSecondary} />
                ) : (
                  <ChevronUp size={18} color={colors.textSecondary} />
                )}
              </Pressable>
              {tasksOpen ? (
                <>
                  {tasks.slice(-5).map(taskRow)}
                  <View style={styles.trayFooter}>
                    <Text style={[styles.footnote, quiet, styles.fill]}>
                      Ending the call leaves accepted tasks running. Stop a task
                      here to cancel it.
                    </Text>
                    <ActionButton
                      label="View in chat"
                      variant="secondary"
                      compact
                      onPress={() => review()}
                    />
                  </View>
                </>
              ) : headline ? (
                taskRow(headline)
              ) : null}
            </View>
          ) : null}
          {/* Grok runs on explicit holds only, so it has no mode to choose. */}
          {option && !isGrok ? (
            <View
              accessibilityRole="radiogroup"
              accessibilityLabel="Listening mode"
              style={[styles.modes, styles.floating, { backgroundColor: surface }]}
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
                      selected && { backgroundColor: main },
                    ]}
                  >
                    <Text
                      style={[
                        styles.modeLabel,
                        { color: selected ? onMain : colors.textSecondary },
                      ]}
                    >
                      {mode === "automatic" ? "Hands-free" : "Hold to talk"}
                    </Text>
                  </Pressable>
                );
              })}
            </View>
          ) : null}
          <View style={styles.controls}>
            {active ? (
              inputMode === "push_to_talk" ? (
                <RoundButton
                  small
                  label={muted ? "Unmute microphone" : "Mute microphone"}
                  surface={muted ? colors.textPrimary : surface}
                  selected={muted}
                  disabled={voice.starting}
                  onPress={() => void voice.mute(!muted)}
                >
                  {muted ? (
                    <MicOff size={20} color={colors.bg} />
                  ) : (
                    <Mic size={20} color={colors.textPrimary} />
                  )}
                </RoundButton>
              ) : (
                <RoundButton
                  small
                  label="Show chat"
                  surface={surface}
                  onPress={() => review()}
                >
                  <MessageSquareText size={20} color={colors.textPrimary} />
                </RoundButton>
              )
            ) : (
              <View style={styles.smallSpacer} />
            )}
            {!active ? (
              <MainButton
                label={voice.error ? "Try again" : "Start voice"}
                fill={main}
                disabled={!canStart}
                onPress={() => void start()}
              >
                <Mic size={30} color={onMain} />
              </MainButton>
            ) : inputMode === "push_to_talk" ? (
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
                    backgroundColor: voice.holding ? colors.primary : main,
                    opacity: microphoneBlocked || !voice.connected ? 0.4 : 1,
                    transform: [{ scale: voice.holding ? 1.08 : 1 }],
                  },
                ]}
              >
                <Mic size={30} color={voice.holding ? colors.onPrimary : onMain} />
              </Pressable>
            ) : (
              <MainButton
                label={muted ? "Unmute microphone" : "Mute microphone"}
                fill={muted ? colors.danger : main}
                disabled={voice.starting}
                onPress={() => void voice.mute(!muted)}
              >
                {muted ? (
                  <MicOff size={30} color={colors.onPrimary} />
                ) : (
                  <Mic size={30} color={onMain} />
                )}
              </MainButton>
            )}
            <RoundButton
              small
              label={active ? "End voice and close" : "Close voice"}
              surface={surface}
              onPress={close}
            >
              <X size={20} color={active ? colors.danger : colors.textPrimary} />
            </RoundButton>
          </View>
          <View style={styles.footer}>
            {active ? (
              <>
                <Text style={[styles.footnote, quiet]}>
                  {inputMode === "automatic"
                    ? "Your microphone is on while unmuted. Use headphones for the clearest audio."
                    : "Audio is sent only while holding the button. Use headphones if you hear echo."}
                  {option && (option.pricing || option.key_source === "own")
                    ? ` ${voiceTariff(option)}`
                    : ""}
                </Text>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel="Resume audio"
                  disabled={voice.starting}
                  onPress={() => void voice.resumeAudio()}
                  style={styles.textButton}
                >
                  <Text style={[styles.footnote, quiet]}>
                    No sound? Resume audio
                  </Text>
                </Pressable>
              </>
            ) : option ? (
              <>
                <Text style={[styles.footnote, quiet]}>
                  {voiceTariff(option)}{" "}
                  {option.billing_owner === "acting_person"
                    ? "Billed to your account."
                    : `Billed to ${option.billing_owner}.`}
                </Text>
                {pricingOpen ? (
                  <Text style={[styles.footnote, quiet]}>
                    {option.key_source === "platform"
                      ? "Platform connection."
                      : "Your provider connection."}
                    {option.pricing
                      ? " Starting a call reserves the first 30 seconds of usage plus any start fee. Applicable benefits are applied first."
                      : ""}{" "}
                    Silence and mute still count toward call time. Agent tasks
                    are billed separately. Microphone access is requested when
                    you start. NyxID saves the text conversation, not an audio
                    recording.
                  </Text>
                ) : null}
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={
                    pricingOpen ? "Hide voice billing details" : "Show voice billing details"
                  }
                  accessibilityState={{ expanded: pricingOpen }}
                  onPress={() => setPricingOpen((value) => !value)}
                  style={styles.textButton}
                >
                  <Text style={[styles.footnote, styles.footnoteLink, { color: colors.textSecondary }]}>
                    {pricingOpen ? "Less" : "How voice is billed"}
                  </Text>
                </Pressable>
              </>
            ) : (
              <Text style={[styles.footnote, quiet]}>
                Microphone access is requested when you start. NyxID saves the
                text conversation, not an audio recording.
              </Text>
            )}
          </View>
        </View>
      </Modal>
    </>
  );
}

// The voice page's pastel wash: sky to lilac to blush, deepened for dark mode.
const LIGHT_WASH = ["#CCDDF6", "#E6E4F4", "#F3DAEE"] as const;
const DARK_WASH = ["#141C2A", "#1B1828", "#271B29"] as const;

/** White round button floating on the wash; selected toggles invert. */
function RoundButton({
  label,
  surface,
  selected,
  disabled = false,
  small = false,
  onPress,
  children,
}: {
  label: string;
  surface: string;
  selected?: boolean;
  disabled?: boolean;
  small?: boolean;
  onPress: () => void;
  children: ReactNode;
}) {
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled, selected: !!selected }}
      disabled={disabled}
      hitSlop={small ? 4 : 2}
      onPress={onPress}
      style={({ pressed }) => [
        small ? styles.roundSmall : styles.round,
        styles.floating,
        {
          backgroundColor: surface,
          opacity: disabled ? 0.4 : pressed ? 0.7 : 1,
        },
      ]}
    >
      {children}
    </Pressable>
  );
}

/** The large centre action: Start, then the microphone. */
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
  errorBody: { padding: 20, gap: 16, paddingBottom: 44 },
  screen: { flex: 1, paddingHorizontal: 20, gap: 12, overflow: "hidden" },
  topBar: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    minHeight: 52,
  },
  pageTitle: {
    flex: 1,
    textAlign: "center",
    fontFamily: fonts.displaySemi,
    fontSize: 17,
    lineHeight: 22,
  },
  round: {
    width: 40,
    height: 40,
    borderRadius: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  roundSmall: {
    width: 48,
    height: 48,
    borderRadius: 24,
    alignItems: "center",
    justifyContent: "center",
  },
  roundSpacer: { width: 40, height: 40 },
  smallSpacer: { width: 48, height: 48 },
  floating: {
    shadowColor: "#2A2540",
    shadowOpacity: 0.12,
    shadowRadius: 10,
    shadowOffset: { width: 0, height: 4 },
    elevation: 3,
  },
  main: {
    width: 76,
    height: 76,
    borderRadius: 38,
    alignItems: "center",
    justifyContent: "center",
    shadowColor: "#1A1530",
    shadowOpacity: 0.28,
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    elevation: 6,
  },
  speech: { alignItems: "center", gap: 10 },
  modes: {
    flexDirection: "row",
    alignSelf: "center",
    borderRadius: 20,
    padding: 3,
  },
  mode: {
    minHeight: 36,
    minWidth: 112,
    paddingHorizontal: 14,
    borderRadius: 17,
    alignItems: "center",
    justifyContent: "center",
  },
  modeLabel: { fontFamily: fonts.bodySemi, fontSize: 14, lineHeight: 18 },
  pill: {
    borderRadius: 12,
    paddingHorizontal: 12,
    paddingVertical: 4,
    maxWidth: "70%",
  },
  pillText: { fontFamily: fonts.body, fontSize: 12, lineHeight: 16 },
  statusRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 20,
  },
  status: { fontFamily: fonts.body, fontSize: 13, lineHeight: 18 },
  elapsed: { fontFamily: fonts.mono, fontSize: 12, lineHeight: 18 },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  stage: {
    flex: 1,
    minHeight: 180,
    alignItems: "center",
    justifyContent: "center",
    gap: 28,
  },
  intro: { alignItems: "center", gap: 8, paddingHorizontal: 12 },
  introTitle: {
    fontFamily: fonts.displaySemi,
    fontSize: 26,
    lineHeight: 33,
    textAlign: "center",
  },
  introDetail: {
    fontFamily: fonts.body,
    fontSize: 13,
    lineHeight: 18,
    textAlign: "center",
  },
  transcript: { flexGrow: 0 },
  transcriptContent: { gap: 14, paddingBottom: 4, paddingHorizontal: 8 },
  caption: { gap: 2, alignItems: "center" },
  captionRecent: { gap: 4 },
  speaker: { fontFamily: fonts.body, fontSize: 12, lineHeight: 16 },
  captionText: {
    fontFamily: fonts.displaySemi,
    fontSize: 24,
    lineHeight: 31,
    textAlign: "center",
  },
  captionEarlier: {
    fontFamily: fonts.bodyRegular,
    fontSize: 15,
    lineHeight: 22,
    textAlign: "center",
  },
  tray: { borderRadius: 14, paddingHorizontal: 14, paddingBottom: 8 },
  actions: { borderRadius: 18, paddingTop: 12, paddingHorizontal: 12, paddingBottom: 4 },
  actionsTitle: { paddingHorizontal: 4, paddingBottom: 8 },
  actionsContent: { gap: 10, paddingBottom: 8 },
  trayHeader: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: 44,
  },
  trayTitle: { fontFamily: fonts.bodySemi, fontSize: 14 },
  trayFooter: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingTop: 8,
  },
  taskRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 6,
  },
  taskState: { fontSize: 12 },
  controls: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: 18,
    minHeight: 100,
  },
  footer: { alignItems: "center", gap: 2 },
  footnote: {
    fontFamily: fonts.body,
    fontSize: 12,
    lineHeight: 16,
    textAlign: "center",
  },
  textButton: { minHeight: 44, justifyContent: "center", paddingHorizontal: 8 },
  footnoteLink: { textDecorationLine: "underline" },
  minimized: {
    flexDirection: "row",
    alignItems: "center",
    borderWidth: 1,
    borderRadius: 12,
    marginBottom: 10,
    paddingLeft: 12,
  },
  restore: {
    flex: 1,
    minHeight: 48,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
});
