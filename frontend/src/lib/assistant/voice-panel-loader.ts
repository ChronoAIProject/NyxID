export function loadVoicePanel() {
  return import("@/components/assistant/voice-panel").then((module) => ({
    default: module.VoicePanel,
  }));
}
