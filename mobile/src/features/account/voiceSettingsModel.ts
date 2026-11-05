import type {
  VoiceOption,
  VoicePreferences,
} from "../assistant/voice/voiceContracts";
import { chooseVoiceOption } from "../assistant/voice/voiceSelection";

export type VoiceSettingsView = {
  option: VoiceOption;
  preferences: VoicePreferences;
  /** Grok's realtime transport only supports Hold to talk. */
  holdOnly: boolean;
};

/**
 * What Settings › Voice shows and saves: the same automatic voice source a
 * call uses (no source chooser on native), with the person's saved voice,
 * conversation mode and completion notice applied where that source allows.
 * Language is not offered: the providers do not receive it yet.
 */
export function voiceSettingsView(
  options: readonly VoiceOption[],
  saved: VoicePreferences | null | undefined,
): VoiceSettingsView | null {
  const option = chooseVoiceOption(options);
  if (!option) return null;
  const holdOnly = option.voice.protocol === "xai_realtime";
  const voice =
    option.voice.voices.find((row) => row.id === saved?.voice)?.id ??
    option.voice.voices[0]?.id ??
    null;
  return {
    option,
    holdOnly,
    preferences: {
      service_id: option.service_id,
      connection_id: option.connection_id,
      key_source: option.key_source,
      model: option.model,
      voice,
      input_mode: holdOnly ? "push_to_talk" : (saved?.input_mode ?? "push_to_talk"),
      language: saved?.language ?? null,
      notify_on_completion: saved?.notify_on_completion ?? false,
    },
  };
}
