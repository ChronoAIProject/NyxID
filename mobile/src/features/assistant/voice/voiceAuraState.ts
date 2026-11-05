import type { VoicePreferences, VoiceSnapshot } from "./voiceContracts";

/** What the call is doing right now; the status line names it, the aura shows it. */
export type VoiceAuraPhase =
  | "resting"
  | "connecting"
  | "listening"
  | "thinking"
  | "speaking"
  | "muted";

export type VoiceAuraInput = {
  starting: boolean;
  connected: boolean;
  inputMode: VoicePreferences["input_mode"];
  holding: boolean;
  /** Server or local state that keeps audio from being sent. */
  microphoneBlocked: boolean;
  muted: boolean;
  /** Playback loudness has been above the speaking threshold recently. */
  outputActive: boolean;
  captions: readonly Pick<
    VoiceSnapshot["captions"][number],
    "speaker" | "complete"
  >[];
  tasks: readonly Pick<VoiceSnapshot["tasks"][number], "state">[];
};

export function voiceAuraPhase(input: VoiceAuraInput): VoiceAuraPhase {
  if (input.starting) return "connecting";
  if (!input.connected) return "resting";
  const last = input.captions[input.captions.length - 1];
  if (input.outputActive || (last?.speaker === "assistant" && !last.complete))
    return "speaking";
  const micOpen =
    !input.microphoneBlocked &&
    (input.inputMode === "automatic" || input.holding);
  if (micOpen && (input.holding || (last?.speaker === "user" && !last.complete)))
    return "listening";
  if (
    (last?.speaker === "user" && last.complete) ||
    input.tasks.some((task) => task.state === "queued" || task.state === "claimed")
  )
    return "thinking";
  if (micOpen) return "listening";
  if (input.muted) return "muted";
  return "resting";
}

export function voiceAuraStatus(
  phase: VoiceAuraPhase,
  inputMode: VoicePreferences["input_mode"],
) {
  switch (phase) {
    case "connecting":
      return "Connecting…";
    case "listening":
      return "Listening";
    case "thinking":
      return "Thinking";
    case "speaking":
      return "Speaking";
    case "muted":
      return "Muted";
    default:
      return inputMode === "push_to_talk" ? "Ready" : "Connected";
  }
}

/**
 * How each phase looks. `gain` is how far the loudness of `source` grows the
 * aura; `orbit` is how far the blobs travel from the centre. Reduce Motion
 * keeps only the opacity: no breathing, orbit or level-driven scale.
 */
export type VoiceAuraTargets = {
  opacity: number;
  scale: number;
  gain: number;
  orbit: number;
  source: "input" | "output";
  breathe: boolean;
};

const TARGETS: Record<VoiceAuraPhase, VoiceAuraTargets> = {
  resting: { opacity: 0.95, scale: 0.92, gain: 0, orbit: 14, source: "input", breathe: true },
  connecting: { opacity: 0.8, scale: 0.88, gain: 0, orbit: 18, source: "input", breathe: true },
  listening: { opacity: 1, scale: 0.97, gain: 0.22, orbit: 12, source: "input", breathe: true },
  thinking: { opacity: 0.95, scale: 0.93, gain: 0, orbit: 26, source: "input", breathe: true },
  speaking: { opacity: 1, scale: 1, gain: 0.26, orbit: 14, source: "output", breathe: true },
  muted: { opacity: 0.45, scale: 0.86, gain: 0, orbit: 6, source: "input", breathe: false },
};

export function voiceAuraTargets(
  phase: VoiceAuraPhase,
  reducedMotion: boolean,
): VoiceAuraTargets {
  const target = TARGETS[phase];
  return reducedMotion
    ? { ...target, scale: 1, gain: 0, orbit: 0, breathe: false }
    : target;
}

/**
 * Fast attack, slow release: the aura eases toward each new level on the UI
 * thread, so speech onsets show at once and fade gracefully without jitter.
 */
export function levelTransitionMs(previous: number, target: number) {
  return target > previous ? 90 : 320;
}

export const OUTPUT_SPEAKING_ON = 0.12;
export const OUTPUT_SPEAKING_OFF = 0.05;
export const OUTPUT_SPEAKING_RELEASE_MS = 600;

export type OutputActivity = { active: boolean; quietSince?: number };

/** Hysteresis so short gaps between words do not flip the phase. */
export function nextOutputActivity(
  state: OutputActivity,
  level: number,
  now: number,
): OutputActivity {
  if (level >= OUTPUT_SPEAKING_ON) return { active: true };
  if (!state.active) return state;
  if (level > OUTPUT_SPEAKING_OFF) return { active: true };
  const quietSince = state.quietSince ?? now;
  return now - quietSince >= OUTPUT_SPEAKING_RELEASE_MS
    ? { active: false }
    : { active: true, quietSince };
}
