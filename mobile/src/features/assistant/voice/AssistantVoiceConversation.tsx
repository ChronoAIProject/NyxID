import { useNativeSheetDismiss } from "../../../components/workspace/useNativeSheetDismiss";
import { useReducedMotion } from "../../../lib/accessibility/useReducedMotion";
import { useEffect, useMemo, useState } from "react";
import {
  ActivityIndicator,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  Switch,
  Text,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useQuery } from "@tanstack/react-query";
import * as WebBrowser from "expo-web-browser";
import {
  ChevronDown,
  ChevronLeft,
  Mic,
  MicOff,
  Minus,
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
import { VoiceMediaBridge } from "./VoiceMediaBridge";
import { chooseVoiceOption, voiceElapsed, voiceTariff } from "./voiceSelection";
import type { VoicePreferences } from "./voiceContracts";

type Props = {
  thread: string;
  agentName: string;
  onClose: () => void;
  onUpdate: () => void;
  onReview: (acknowledgementId?: string) => void;
};
type Picker = "voice" | "mode";

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
        <View style={styles.body}>
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
  const { colors } = useTheme();
  const sheet = useNativeSheetDismiss(onClose);
  const reducedMotion = useReducedMotion();
  const focused = useIsFocused();
  const { isConnected } = useNetworkStatus();
  const voice = useNativeVoice(thread, environment, onUpdate);
  const [minimized, setMinimized] = useState(false);
  const [picker, setPicker] = useState<Picker>();
  const [preferredVoice, setPreferredVoice] = useState<string>();
  const [mode, setMode] = useState<VoicePreferences["input_mode"]>();
  const [notify, setNotify] = useState<boolean>();
  const [expandedCaptions, setExpandedCaptions] = useState(false);
  const [showErrorDetails, setShowErrorDetails] = useState(false);
  const [billingOpenError, setBillingOpenError] = useState(false);
  const [now, setNow] = useState(Date.now());
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
  const inputMode = isGrok
    ? "push_to_talk"
    : (mode ?? saved?.input_mode ?? "push_to_talk");
  const voiceId =
    option?.voice.voices.find(
      (row) => row.id === (preferredVoice ?? saved?.voice),
    )?.id ??
    option?.voice.voices[0]?.id ??
    null;
  const voiceLabel = option?.voice.voices.find(
    (row) => row.id === voiceId,
  )?.label;
  const notifyOnCompletion = notify ?? saved?.notify_on_completion ?? false;
  const active = voice.starting || voice.connected;
  const session = voice.snapshot?.session;
  const muted = session?.muted ?? false;
  const microphoneBlocked =
    muted ||
    session?.input_muted ||
    session?.end_requested ||
    session?.state !== "active";
  const status = voice.starting
    ? "Connecting…"
    : voice.connected
      ? session?.state === "closing"
        ? "Ending…"
        : "Connected"
      : voice.recovery || (voice.error && !voice.snapshot)
        ? "Unable to start"
        : voice.conflict
          ? "Call already active"
          : voice.snapshot
            ? "Call ended"
            : "Ready to talk";
  const duration = voiceElapsed(session?.created_at, session?.closed_at, now);
  const captions = voice.snapshot?.captions ?? [];
  const shownCaptions = expandedCaptions ? captions : captions.slice(-2);
  const tasks = voice.snapshot?.tasks ?? [];
  const canStart =
    !!option?.available &&
    !active &&
    !voice.conflict &&
    isConnected &&
    options.isSuccess &&
    settings.isSuccess;
  const theme = useMemo(
    () => ({ color: colors.textPrimary }),
    [colors.textPrimary],
  );
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
  async function start() {
    if (!canStart || !option) return;
    setBillingOpenError(false);
    await voice.start(
      {
        service_id: option.service_id,
        connection_id: option.connection_id,
        key_source: option.key_source,
        model: option.model,
        voice: voiceId,
        input_mode: inputMode,
        language: saved?.language ?? null,
        notify_on_completion: notifyOnCompletion,
      },
      option.voice.protocol,
    );
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
  const pickerRows =
    picker === "voice"
      ? (option?.voice.voices ?? []).map((row) => ({
          id: row.id,
          title: row.label,
          detail: "",
          selected: row.id === voiceId,
          disabled: false,
          choose: () => setPreferredVoice(row.id),
        }))
      : [
          {
            id: "push_to_talk",
            title: "Hold to talk",
            detail:
              "Your microphone sends audio only while you hold the button.",
            selected: inputMode === "push_to_talk",
            disabled: false,
            choose: () => setMode("push_to_talk"),
          },
          ...(!isGrok
            ? [
                {
                  id: "automatic",
                  title: "Automatic",
                  detail:
                    "Keep talking naturally. Your microphone stays on during the call.",
                  selected: inputMode === "automatic",
                  disabled: false,
                  choose: () => setMode("automatic" as const),
                },
              ]
            : []),
        ];
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
            <Label>
              {status} · {agentName}
              {active ? ` · ${duration}` : ""}
            </Label>
            <ChevronDown size={18} color={colors.textSecondary} />
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
      <Modal
        {...sheet.modalProps}
        visible={!minimized && sheet.modalProps.visible}
        transparent
        animationType={reducedMotion ? "none" : "slide"}
        presentationStyle="overFullScreen"
        onRequestClose={() => (picker ? setPicker(undefined) : close())}
      >
        <WorkspaceSheetSurface nativePresentation onClose={close}>
          <View style={[styles.header, { borderBottomColor: colors.border }]}>
            {picker ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Back to voice call"
                onPress={() => setPicker(undefined)}
                style={styles.iconButton}
              >
                <ChevronLeft size={22} color={colors.textSecondary} />
              </Pressable>
            ) : null}
            <Text accessibilityRole="header" style={[styles.heading, theme]}>
              {picker
                ? picker === "voice"
                  ? "Voice"
                  : "Microphone mode"
                : "Voice conversation"}
            </Text>
            {!picker && active ? (
              <Pressable
                accessibilityRole="button"
                accessibilityLabel="Minimize voice call"
                onPress={minimize}
                style={styles.iconButton}
              >
                <Minus size={22} color={colors.textSecondary} />
              </Pressable>
            ) : null}
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={
                active ? "End voice and close" : "Close voice"
              }
              onPress={close}
              style={styles.iconButton}
            >
              <X size={22} color={colors.textSecondary} />
            </Pressable>
          </View>
          <ScrollView
            style={styles.fill}
            contentContainerStyle={styles.body}
            keyboardShouldPersistTaps="handled"
          >
            {picker ? (
              <>
                {pickerRows.map((row) => (
                  <Pressable
                    key={row.id}
                    accessibilityRole="radio"
                    accessibilityState={{
                      checked: !!row.selected,
                      disabled: row.disabled,
                    }}
                    disabled={row.disabled}
                    accessibilityLabel={
                      row.title + (row.detail ? `. ${row.detail}` : "")
                    }
                    onPress={() => {
                      row.choose();
                      setPicker(undefined);
                    }}
                    style={[
                      styles.choice,
                      {
                        backgroundColor: row.selected
                          ? colors.primaryTone.bg
                          : colors.card,
                        borderColor: row.selected
                          ? colors.primaryTone.border
                          : colors.border,
                      },
                    ]}
                  >
                    <Label style={styles.choiceTitle}>{row.title}</Label>
                    {row.detail ? <Label muted>{row.detail}</Label> : null}
                  </Pressable>
                ))}
              </>
            ) : (
              <>
                <View style={styles.hero}>
                  <View
                    style={[
                      styles.orb,
                      {
                        backgroundColor: voice.holding
                          ? colors.primary
                          : colors.primaryTone.bg,
                      },
                    ]}
                  >
                    {voice.starting ? (
                      <ActivityIndicator size="large" color={colors.primary} />
                    ) : muted ? (
                      <MicOff size={34} color={colors.primaryOnTint} />
                    ) : (
                      <Mic
                        size={34}
                        color={
                          voice.holding
                            ? colors.onPrimary
                            : colors.primaryOnTint
                        }
                      />
                    )}
                  </View>
                  <Text style={[styles.agentTitle, theme]}>
                    Talk with {agentName}
                  </Text>
                  <Label muted>Uses this chat’s context</Label>
                  <Text
                    accessibilityLiveRegion="polite"
                    style={[
                      styles.status,
                      {
                        color: voice.connected
                          ? colors.success
                          : voice.error
                            ? colors.danger
                            : colors.textSecondary,
                      },
                    ]}
                  >
                    {status}
                    {active ? ` · ${duration}` : ""}
                  </Text>
                </View>
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
                        showErrorDetails
                          ? "Hide error details"
                          : "Show error details"
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
                  <Notice message="The call will end soon because you haven’t spoken. Hold to talk to continue." />
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
                    <Card>
                      <SettingRow
                        label="Voice"
                        value={voiceLabel ?? "Choose a voice"}
                        onPress={() => setPicker("voice")}
                        disabled={!option}
                      />
                      <SettingRow
                        label="Microphone"
                        value={
                          inputMode === "automatic"
                            ? "Automatic"
                            : "Hold to talk"
                        }
                        onPress={() => setPicker("mode")}
                        disabled={!option}
                      />
                      <View style={styles.switchRow}>
                        <View style={styles.fill}>
                          <Label>Notify when tasks finish</Label>
                          <Label muted>
                            Work can continue after the call ends.
                          </Label>
                        </View>
                        <Switch
                          accessibilityLabel="Notify when voice tasks finish"
                          value={notifyOnCompletion}
                          onValueChange={setNotify}
                          trackColor={{ true: colors.primary }}
                        />
                      </View>
                    </Card>
                    {option ? (
                      <>
                        {!option.available ? (
                          <Notice
                            message={
                              option.unavailable_reason ??
                              "Voice is unavailable. Reconnect your provider and try again."
                            }
                          />
                        ) : null}
                        <Label muted>
                          {option.key_source === "platform"
                            ? "Platform connection"
                            : "Your provider connection"}{" "}
                          ·{" "}
                          {option.billing_owner === "acting_person"
                            ? "Billed to your account"
                            : `Billed to ${option.billing_owner}`}
                        </Label>
                        <Label muted>{voiceTariff(option)}</Label>
                        {option.pricing ? (
                          <Label muted>
                            Starting a call reserves the first 30 seconds of
                            usage plus any start fee. Applicable benefits are
                            applied first.
                          </Label>
                        ) : null}
                        <Label muted>
                          Silence and mute still count toward call time. Agent
                          tasks are billed separately.
                        </Label>
                      </>
                    ) : null}
                    <Label muted>
                      Microphone access is requested when you start. NyxID saves
                      the text conversation, not an audio recording.
                    </Label>
                  </>
                ) : (
                  <>
                    {voice.connected && inputMode === "push_to_talk" ? (
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel="Hold to talk"
                        accessibilityState={{
                          disabled: !!microphoneBlocked,
                          selected: voice.holding,
                        }}
                        disabled={!!microphoneBlocked}
                        onPressIn={() => voice.hold(true)}
                        onPressOut={() => voice.hold(false)}
                        onAccessibilityAction={({ nativeEvent }) => {
                          if (nativeEvent.actionName === "activate")
                            voice.hold(!voice.holding);
                        }}
                        accessibilityActions={[
                          {
                            name: "activate",
                            label: voice.holding
                              ? "Stop talking"
                              : "Start talking",
                          },
                        ]}
                        style={[
                          styles.holdButton,
                          {
                            backgroundColor: voice.holding
                              ? colors.primary
                              : colors.primaryTone.bg,
                            borderColor: colors.primaryTone.border,
                            opacity: microphoneBlocked ? 0.4 : 1,
                          },
                        ]}
                      >
                        <Mic
                          size={22}
                          color={
                            voice.holding
                              ? colors.onPrimary
                              : colors.primaryOnTint
                          }
                        />
                        <Text
                          style={[
                            styles.holdText,
                            {
                              color: voice.holding
                                ? colors.onPrimary
                                : colors.primaryOnTint,
                            },
                          ]}
                        >
                          {voice.holding ? "Release to send" : "Hold to talk"}
                        </Text>
                      </Pressable>
                    ) : null}
                    <View style={styles.controls}>
                      <CallControl
                        label={muted ? "Unmute microphone" : "Mute microphone"}
                        disabled={voice.starting}
                        onPress={() => void voice.mute(!muted)}
                      >
                        {muted ? (
                          <MicOff size={21} color={colors.textPrimary} />
                        ) : (
                          <Mic size={21} color={colors.textPrimary} />
                        )}
                      </CallControl>
                      <CallControl
                        label={
                          voice.speakerMuted ? "Enable speaker" : "Mute speaker"
                        }
                        disabled={voice.starting}
                        onPress={() => voice.muteSpeaker(!voice.speakerMuted)}
                      >
                        {voice.speakerMuted ? (
                          <VolumeX size={21} color={colors.textPrimary} />
                        ) : (
                          <Volume2 size={21} color={colors.textPrimary} />
                        )}
                      </CallControl>
                    </View>
                    <ActionButton
                      label="Resume audio"
                      variant="secondary"
                      disabled={voice.starting}
                      onPress={() => void voice.resumeAudio()}
                    />
                    <ActionButton
                      label="End voice session"
                      variant="danger"
                      onPress={() => void voice.end()}
                    />
                    <Label muted>
                      {inputMode === "automatic"
                        ? "Your microphone is on while unmuted. Use headphones for the clearest audio."
                        : "Audio is sent only while holding the button. Use headphones if you hear echo."}
                    </Label>
                  </>
                )}
                {captions.length ? (
                  <Card>
                    <View style={styles.sectionRow}>
                      <Label style={styles.choiceTitle}>Transcript</Label>
                      <Pressable
                        accessibilityRole="button"
                        accessibilityLabel={
                          expandedCaptions
                            ? "Show latest captions"
                            : "Show full voice transcript"
                        }
                        onPress={() => setExpandedCaptions(!expandedCaptions)}
                        style={styles.textButton}
                      >
                        <Label style={{ color: colors.primaryOnTint }}>
                          {expandedCaptions ? "Show latest" : "Show all"}
                        </Label>
                      </Pressable>
                    </View>
                    {shownCaptions.map((caption) => (
                      <View key={caption.id} style={styles.caption}>
                        <Label muted>
                          {caption.speaker === "user" ? "You" : agentName}
                          {caption.complete ? "" : " · Speaking"}
                        </Label>
                        <Text selectable style={[styles.captionText, theme]}>
                          {caption.text}
                        </Text>
                      </View>
                    ))}
                  </Card>
                ) : null}
                {tasks.length ? (
                  <Card>
                    <Label style={styles.choiceTitle}>Agent tasks</Label>
                    {tasks.slice(-5).map((task) => (
                      <View
                        key={task.id}
                        style={[styles.task, { borderColor: colors.border }]}
                      >
                        <Label>{task.title || "Agent task"}</Label>
                        <Label muted>{task.state.replaceAll("_", " ")}</Label>
                        {task.state === "awaiting_confirmation" ? (
                          <ActionButton
                            label="Review in chat"
                            variant="secondary"
                            onPress={() =>
                              review(task.pending_acknowledgement_ids[0])
                            }
                          />
                        ) : null}
                        {task.state === "queued" || task.state === "claimed" ? (
                          <ActionButton
                            label="Stop task"
                            variant="secondary"
                            onPress={() => void voice.stopTask(task.id)}
                          />
                        ) : null}
                      </View>
                    ))}
                    <ActionButton
                      label="View in chat"
                      variant="secondary"
                      onPress={() => review()}
                    />
                    <Label muted>
                      Ending the call leaves accepted tasks running. Stop a task
                      here to cancel it.
                    </Label>
                  </Card>
                ) : null}
              </>
            )}
          </ScrollView>
          {!picker && !active ? (
            <View
              style={[styles.startActions, { borderTopColor: colors.border }]}
            >
              <ActionButton
                label={voice.error ? "Try again" : "Start voice"}
                disabled={!canStart}
                onPress={() => void start()}
              />
            </View>
          ) : null}
        </WorkspaceSheetSurface>
      </Modal>
    </>
  );
}

function SettingRow({
  label,
  value,
  disabled,
  onPress,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={`${label}: ${value}`}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={styles.settingRow}
    >
      <View style={styles.fill}>
        <Label muted>{label}</Label>
        <Label>{value}</Label>
      </View>
      <ChevronDown size={18} color={colors.textMuted} />
    </Pressable>
  );
}
function CallControl({
  label,
  disabled,
  onPress,
  children,
}: {
  label: string;
  disabled: boolean;
  onPress: () => void;
  children: React.ReactNode;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={[
        styles.control,
        {
          backgroundColor: colors.cardSoft,
          borderColor: colors.border,
          opacity: disabled ? 0.4 : 1,
        },
      ]}
    >
      {children}
      <Label>{label}</Label>
    </Pressable>
  );
}
const styles = StyleSheet.create({
  fill: { flex: 1 },
  body: { padding: 20, gap: 16, paddingBottom: 44 },
  startActions: {
    paddingHorizontal: 20,
    paddingTop: 12,
    paddingBottom: 34,
    borderTopWidth: StyleSheet.hairlineWidth,
  },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: 12,
    minHeight: 52,
    borderBottomWidth: StyleSheet.hairlineWidth,
  },
  heading: {
    flex: 1,
    fontFamily: fonts.displaySemi,
    fontSize: 19,
    paddingLeft: 8,
  },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  hero: { alignItems: "center", gap: 8, paddingVertical: 12 },
  orb: {
    width: 88,
    height: 88,
    borderRadius: 44,
    alignItems: "center",
    justifyContent: "center",
    marginBottom: 8,
  },
  agentTitle: { fontFamily: fonts.display, fontSize: 24, textAlign: "center" },
  status: { fontFamily: fonts.bodySemi, fontSize: 14, marginTop: 4 },
  settingRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
    paddingVertical: 10,
    minHeight: 56,
  },
  switchRow: {
    flexDirection: "row",
    gap: 12,
    alignItems: "center",
    paddingVertical: 8,
  },
  choice: {
    gap: 6,
    borderWidth: 1,
    borderRadius: 8,
    padding: 16,
    minHeight: 56,
  },
  choiceTitle: { fontFamily: fonts.bodySemi, fontSize: 15 },
  holdButton: {
    minHeight: 72,
    borderWidth: 1,
    borderRadius: 12,
    gap: 10,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "center",
  },
  holdText: { fontFamily: fonts.bodySemi, fontSize: 17 },
  controls: { flexDirection: "row", gap: 12 },
  control: {
    flex: 1,
    minHeight: 80,
    gap: 6,
    padding: 10,
    borderWidth: 1,
    borderRadius: 8,
    alignItems: "center",
    justifyContent: "center",
  },
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
  sectionRow: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  textButton: { minHeight: 44, justifyContent: "center", paddingHorizontal: 8 },
  caption: { gap: 4, paddingVertical: 8 },
  captionText: { fontFamily: fonts.bodyRegular, fontSize: 15, lineHeight: 23 },
  task: {
    gap: 8,
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingTop: 12,
    marginTop: 12,
  },
});
