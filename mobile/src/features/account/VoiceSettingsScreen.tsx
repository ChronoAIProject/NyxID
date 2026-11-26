import { translateApp } from "../../i18n/appTranslation";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useMemo, useState } from "react";
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
import {
  AUTOMATIC_DICTATION_LANGUAGE,
  useDictationLanguage,
} from "../assistant/voice/useDictationLanguage";
import { DictationLanguagePicker } from "../assistant/voice/DictationLanguagePicker";
import { voiceSettingsView } from "./voiceSettingsModel";

const MODES = [
  {
    value: "push_to_talk",
    get label() {
      return translateApp("Hold to talk");
    },
    get detail() {
      return translateApp("NyxBot listens while you hold the button.");
    },
  },
  {
    value: "automatic",
    get label() {
      return translateApp("Hands-free");
    },
    get detail() {
      return translateApp("NyxBot listens and answers when you pause.");
    },
  },
] as const;

/**
 * Settings › Voice: how live voice conversations sound and listen, and the
 * composer's dictation language. Calls preserve the saved voice source; only
 * what the person can really change is offered. Accounts
 * without live voice see only Dictation.
 */
export function VoiceSettingsScreen({
  route,
}: SheetScreenProps<"VoiceSettings">) {
  useAppLanguage();
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const voiceEnabled = route?.params?.voice ?? true;
  const language = useDictationLanguage(true);
  const [languagePicker, setLanguagePicker] = useState(false);
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
  const selectionReady = options.isSuccess && settings.isSuccess;
  const view =
    selectionReady && options.data && settings.data
      ? voiceSettingsView(options.data.options, settings.data.voice)
      : null;
  const controlsDisabled =
    !selectionReady || !environment || !isConnected || save.isPending;
  const update = (change: Partial<VoicePreferences>) => {
    if (!voiceEnabled || !view || controlsDisabled) return;
    if (
      change.voice !== undefined &&
      !view.option.voice.voices.some((row) => row.id === change.voice)
    )
      return;
    if (view.missingVoice && change.voice === undefined) return;
    save.mutate({ ...view.preferences, ...change });
  };
  const loading =
    voiceEnabled && (options.isPending || settings.isPending) && isConnected;
  const error = options.error ?? settings.error;
  const voices = view?.option.voice.voices ?? [];
  return (
    <SettingsPage>
      {voiceEnabled && !isConnected && (
        <Notice
          message={t("You're offline. Reconnect to change voice settings.")}
        />
      )}
      {!voiceEnabled ? null : loading ? (
        <SkeletonRows
          rows={3}
          title
          accessibilityLabel={t("Loading voice settings")}
        />
      ) : error || !environment ? (
        <>
          <Notice
            tone="error"
            message={
              error instanceof Error
                ? error.message
                : t("Voice settings could not be loaded.")
            }
          />
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={t("Retry")}
              onPress={() => {
                void options.refetch();
                void settings.refetch();
              }}
            />
          </SettingsGroup>
        </>
      ) : view ? (
        <SettingsGroup
          title={t("Conversation")}
          footer={translateApp("Used for live voice conversations with NyxBot. {{voiceTariff}}", { voiceTariff: voiceTariff(view.option)})}
        >
          <SettingsMenuRow
            icon={AudioLines}
            label={t("Voice")}
            detail={
              view.missingVoice
                ? t(
                    "Your saved voice ({{voice}}) is no longer available. Choose a voice to continue.",
                    { voice: view.missingVoice },
                  )
                : undefined
            }
            value={view.preferences.voice ?? ""}
            options={voices.map((row) => ({
              value: row.id,
              label: row.label,
            }))}
            disabled={controlsDisabled || !voices.length}
            onChange={(voice) => update({ voice })}
          />
          <SettingsMenuRow
            icon={Mic}
            label={t("Listening")}
            detail={
              view.holdOnly
                ? t("This voice service supports Hold to talk only.")
                : undefined
            }
            value={view.preferences.input_mode}
            options={MODES.map((mode) => ({
              ...mode,
              label: t(mode.label),
              detail: t(mode.detail),
            }))}
            disabled={controlsDisabled || !!view.missingVoice || view.holdOnly}
            onChange={(input_mode) => update({ input_mode })}
          />
          <SettingsRow
            icon={Bell}
            label={t("Notify when tasks finish")}
            detail={t(
              "When a task you asked for by voice completes after the call.",
            )}
            accessory={
              <Switch
                accessibilityLabel={t("Notify when tasks finish")}
                value={view.preferences.notify_on_completion}
                disabled={controlsDisabled || !!view.missingVoice}
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
            label={t("Voice isn't available right now")}
            detail={t(
              "No voice service is ready for your account. Try again later.",
            )}
          />
        </SettingsGroup>
      )}
      {save.error ? (
        <Notice
          tone="error"
          message={
            save.error instanceof Error
              ? save.error.message
              : t("Voice settings could not be loaded.")
          }
        />
      ) : null}
      <SettingsGroup
        title={t("Dictation")}
        footer={t(
          "This language is used for message dictation and crew voice dictation. Voice conversations use their own language preference. Words dictated into a message can be edited before sending.",
        )}
      >
        <SettingsRow
          icon={Languages}
          label={t("Language")}
          value={language.label}
          detail={
            language.explanation ??
            (language.value === AUTOMATIC_DICTATION_LANGUAGE
              ? t("Device language")
              : undefined)
          }
          onPress={() => setLanguagePicker(true)}
        />
      </SettingsGroup>
      {languagePicker && (
        <DictationLanguagePicker
          language={language}
          onClose={() => setLanguagePicker(false)}
        />
      )}
    </SettingsPage>
  );
}
