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
import { chooseVoiceOption, voiceElapsed } from "./voiceSelection";
import type { VoicePreferences } from "./voiceContracts";

type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  minimized?: boolean;
  onExpand?: () => void;
  onUpdate: () => void;
};

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
  const allCaptions = voice.snapshot?.captions ?? [];
  const captions = allCaptions.filter(
    (caption) =>
      caption.speaker === "user" && !caption.sealed && !!caption.text.trim(),
  );
  const tasks = voice.snapshot?.tasks ?? [];
  const phase = voiceAuraPhase({
    starting: voice.starting,
    connected: voice.connected,
    inputMode,
    holding: voice.holding,
    microphoneBlocked,
    muted,
    outputActive: levels.speaking,
    captions: allCaptions,
    tasks,
  });
  const autoStarted = useRef(false);
  const needsRetry =
    (autoStarted.current || !!voice.error || !!voice.snapshot) && !active;
  const status =
    options.error || settings.error
      ? "Unable to load voice"
      : options.isSuccess && (!option || !option.available)
        ? "Voice unavailable"
        : voice.starting ||
            (!autoStarted.current &&
              !voice.snapshot &&
              !voice.error &&
              !voice.conflict)
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
                  : "Ready to retry";
  const duration =
    voice.starting || voice.connectedAt === undefined
      ? "-"
      : voiceElapsed(
          new Date(voice.connectedAt).toISOString(),
          null,
          voice.disconnectedAt ?? now,
        ).padStart(5, "0");
  const canStart =
    !!option?.available &&
    !active &&
    !voice.conflict &&
    isConnected &&
    options.isSuccess &&
    settings.isSuccess;
  useEffect(() => {
    if (
      !canStart ||
      !focused ||
      autoStarted.current ||
      voice.snapshot ||
      voice.error
    )
      return;
    autoStarted.current = true;
    void start();
  }, [canStart, focused]);
  useEffect(() => {
    if (voice.holding) voice.hold(false);
    setHelpOpen(false);
  }, [minimized]);
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
    if (!reducedMotion)
      LayoutAnimation.configureNext(LayoutAnimation.Presets.easeInEaseOut);
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
              compact ? styles.compactMain : styles.main,
              {
                backgroundColor: main,
                opacity: microphoneBlocked || !voice.connected ? 0.4 : 1,
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
                    ? "Your microphone is on while unmuted. NyxBot answers when you pause."
                    : "Audio is sent only while you hold the microphone. Release to send."}{" "}
                  Use headphones if you hear echo.
                </Text>
                <ActionButton
                  label="Resume audio"
                  variant="secondary"
                  compact
                  disabled={!voice.connected}
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
                                ? "NyxBot listens and answers when you pause"
                                : "NyxBot listens while you hold the microphone"
                          }
                          accessibilityState={{
                            selected,
                            checked: selected,
                            disabled: voice.starting || isGrok,
                          }}
                          disabled={voice.starting || isGrok}
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
                  <>
                    <View ref={helpAnchor} collapsable={false}>
                      <IconButton
                        label="Voice help"
                        selected={helpOpen}
                        onPress={openHelp}
                      >
                        <CircleHelp size={18} color={colors.textPrimary} />
                      </IconButton>
                    </View>
                  </>
                ) : null}
              </VoiceComposerHeader>
              <View style={styles.waveRow}>
                <DictationWaveform subscribe={levels.subscribeLevel} />
              </View>
              {!active ||
              captions.length > 0 ||
              voice.error ||
              voice.recovery ||
              voice.conflict ||
              billingOpenError ||
              !isConnected ||
              session?.idle_warning ? (
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
                          <ActionButton
                            label="Continue with text"
                            variant="secondary"
                            onPress={close}
                          />
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
                      {voice.conflict ? (
                        <Card>
                          <Label>
                            A voice call is already open. End it before starting
                            another call here.
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
              <VoiceComposerControls>
                <View style={styles.sideControls}>
                  {active ? (
                    <>
                      <IconButton
                        label={
                          voice.speakerMuted ? "Enable speaker" : "Mute speaker"
                        }
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
                    </>
                  ) : null}
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
                  ) : null}
                </View>
                {!active ? (
                  <View style={styles.holdColumn}>
                    <MainButton
                      label={needsRetry ? "Try again" : "Connecting voice"}
                      fill={main}
                      disabled={!canStart || !needsRetry}
                      onPress={() => void start()}
                    >
                      <Mic size={26} color={onMain} />
                    </MainButton>
                    <Text
                      style={[styles.holdHint, { color: colors.textSecondary }]}
                    >
                      {needsRetry ? "Try again" : "Connecting…"}
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
                      disabled={voice.starting}
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
                    label={active ? "End voice and close" : "Close voice"}
                    onPress={close}
                  >
                    {active && inputMode === "automatic" ? (
                      <PhoneOff size={20} color={colors.danger} />
                    ) : (
                      <X
                        size={20}
                        color={active ? colors.danger : colors.textPrimary}
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
                inputMode === "push_to_talk"
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
              onClose={close}
            />
          )}
        </VoiceComposerShell>
      </Animated.View>
    </>
  );
}

const styles = StyleSheet.create({
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
