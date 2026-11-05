import { useMemo } from "react";
import { Switch } from "react-native";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AudioLines, Bell, Languages, Mic } from "lucide-react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { SkeletonRows } from "../../components/Skeleton";
import {
  SettingsGroup,
  SettingsMenuRow,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { nativeVoiceEnvironment } from "../../lib/api/assistantVoiceApi";
import { useTheme } from "../../theme/ThemeContext";
import type { VoicePreferences } from "../assistant/voice/voiceContracts";
import { voiceTariff } from "../assistant/voice/voiceSelection";
import { useDictationLanguage } from "../assistant/voice/useDictationLanguage";
import { voiceSettingsView } from "./voiceSettingsModel";

const MODES = [
  {
    value: "push_to_talk",
    label: "Hold to talk",
    detail: "NyxBot listens while you hold the button.",
  },
  {
    value: "automatic",
    label: "Hands-free",
    detail: "NyxBot listens and answers when you pause.",
  },
] as const;

const message = (error: unknown) =>
  error instanceof Error ? error.message : "Voice settings could not be loaded.";

/**
 * Settings › Voice: how live voice conversations sound and listen, and the
 * composer's dictation language. The voice source is chosen automatically, as
 * in a call; only what the person can really change is offered. Accounts
 * without live voice see only Dictation.
 */
export function VoiceSettingsScreen({ route }: SheetScreenProps<"VoiceSettings">) {
  const { colors } = useTheme();
  const voiceEnabled = route?.params?.voice ?? true;
  const language = useDictationLanguage(true);
  const queryClient = useQueryClient();
  const { isConnected } = useNetworkStatus();
  const environment = useMemo(() => {
    try {
      return nativeVoiceEnvironment();
    } catch {
      return null;
    }
  }, []);
  const settingsKey = ["workspace", "voice-settings", environment?.epoch];
  const options = useQuery({
    queryKey: ["workspace", "voice-options", "settings", environment?.epoch],
    queryFn: ({ signal }) => environment!.api.options(undefined, signal),
    enabled: voiceEnabled && !!environment && isConnected,
    retry: false,
  });
  const settings = useQuery({
    queryKey: settingsKey,
    queryFn: ({ signal }) => environment!.api.settings(signal),
    enabled: voiceEnabled && !!environment && isConnected,
    retry: false,
  });
  const save = useMutation({
    mutationFn: (preferences: VoicePreferences) =>
      environment!.api.savePreferences(preferences),
    onSuccess: (data) => queryClient.setQueryData(settingsKey, data),
  });
  const view =
    options.data && settings.data
      ? voiceSettingsView(options.data.options, settings.data.voice)
      : null;
  const update = (change: Partial<VoicePreferences>) => {
    if (!view || save.isPending) return;
    save.mutate({ ...view.preferences, ...change });
  };
  const loading =
    voiceEnabled && (options.isPending || settings.isPending) && isConnected;
  const error = options.error ?? settings.error;
  const voices = view?.option.voice.voices ?? [];
  return (
    <SettingsPage>
      {voiceEnabled && !isConnected && (
        <Notice message="You're offline. Reconnect to change voice settings." />
      )}
      {!voiceEnabled ? null : loading ? (
        <SkeletonRows rows={3} title accessibilityLabel="Loading voice settings" />
      ) : error || !environment ? (
        <>
          <Notice tone="error" message={message(error)} />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label="Retry"
              onPress={() => {
                void options.refetch();
                void settings.refetch();
              }}
            />
          </SettingsGroup>
        </>
      ) : view ? (
        <SettingsGroup
          title="Conversation"
          footer={`Used for live voice conversations with NyxBot. ${voiceTariff(view.option)}`}
        >
          <SettingsMenuRow
            icon={AudioLines}
            label="Voice"
            value={view.preferences.voice ?? ""}
            options={voices.map((row) => ({ value: row.id, label: row.label }))}
            disabled={save.isPending || voices.length < 2}
            onChange={(voice) => update({ voice })}
          />
          <SettingsMenuRow
            icon={Mic}
            label="Listening"
            detail={
              view.holdOnly
                ? "This voice service supports Hold to talk only."
                : undefined
            }
            value={view.preferences.input_mode}
            options={MODES}
            disabled={save.isPending || view.holdOnly}
            onChange={(input_mode) => update({ input_mode })}
          />
          <SettingsRow
            icon={Bell}
            label="Notify when tasks finish"
            detail="When a task you asked for by voice completes after the call."
            accessory={
              <Switch
                accessibilityLabel="Notify when tasks finish"
                value={view.preferences.notify_on_completion}
                disabled={save.isPending}
                onValueChange={(notify_on_completion) =>
                  update({ notify_on_completion })
                }
                trackColor={{ false: colors.borderSoft, true: colors.success }}
              />
            }
          />
        </SettingsGroup>
      ) : (
        <SettingsGroup inset={false}>
          <SettingsRow
            label="Voice isn't available right now"
            detail="No voice service is ready for your account. Try again later."
          />
        </SettingsGroup>
      )}
      {save.error ? (
        <Notice tone="error" message={message(save.error)} />
      ) : null}
      <SettingsGroup
        title="Dictation"
        footer="The microphone in the message field uses your iPhone's speech recognition, on the device when available. The words appear in your message, where you can edit them before sending."
      >
        <SettingsMenuRow
          icon={Languages}
          label="Language"
          value={language.value}
          options={language.options}
          onChange={language.choose}
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
