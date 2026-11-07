import type {
  VoiceOption,
  VoicePreferences,
} from "../assistant/voice/voiceContracts";
import { chooseVoiceOption } from "../assistant/voice/voiceSelection";

export type VoiceSettingsView = {
  option: VoiceOption;
  preferences: VoicePreferences;
  /** Keep a removed saved voice visible until the person explicitly replaces it. */
  missingVoice: string | null;
  /** Grok's realtime transport only supports Hold to talk. */
  holdOnly: boolean;
};

/**
 * What Settings › Voice shows and saves: the same saved source or first-use
 * default a call uses (no source chooser on native), with the person's voice,
 * conversation mode and completion notice applied where that source allows.
 * Language is not offered: the providers do not receive it yet.
 */
export function voiceSettingsView(
  options: readonly VoiceOption[],
  saved: VoicePreferences | null | undefined,
): VoiceSettingsView | null {
  const option = chooseVoiceOption(options, saved);
  if (!option?.available) return null;
  const holdOnly = option.voice.protocol === "xai_realtime";
  const voice = saved?.voice ?? option.voice.voices[0]?.id ?? null;
  const missingVoice =
    voice && !option.voice.voices.some((row) => row.id === voice)
      ? voice
      : null;
  return {
    option,
    holdOnly,
    missingVoice,
    preferences: {
      service_id: option.service_id,
      connection_id: option.connection_id,
      key_source: option.key_source,
      model: option.model,
      voice,
      input_mode: holdOnly
        ? "push_to_talk"
        : (saved?.input_mode ?? "push_to_talk"),
      language: saved?.language ?? null,
      notify_on_completion: saved?.notify_on_completion ?? false,
    },
  };
}
