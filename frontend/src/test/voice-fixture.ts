import type { VoiceOption, VoicePreferences } from "@/schemas/assistant-voice";
/** Test-only response. No provider key, network spend or claimed provider entitlement. */
export const voiceOption: VoiceOption = {
  service_id: "11111111-1111-4111-8111-111111111111",
  connection_id: null,
  key_source: "platform",
  model: "gpt-live-1",
  model_label: "OpenAI · gpt-live-transcribe",
  default_model: true,
  available: true,
  unavailable_reason: null,
  billing_owner: "acting_person",
  voice: {
    protocol: "openai_live",
    models: [{ id: "gpt-live-1", label: "Live" }],
    voices: [{ id: "marin", label: "Marin" }],
    usage_source: "provider_reported",
    billing_metrics: ["voice_seconds"],
  },
  pricing: {
    metric: "voice_seconds",
    credits_per_unit: "0.01",
    sync_status: "synced",
    components: [],
  },
};
export const voicePreferences: VoicePreferences = {
  service_id: voiceOption.service_id,
  connection_id: null,
  key_source: "platform",
  model: voiceOption.model,
  voice: null,
  input_mode: "push_to_talk",
  language: null,
  notify_on_completion: false,
};
