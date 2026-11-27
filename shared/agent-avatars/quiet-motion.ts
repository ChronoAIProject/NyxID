import type { RosterActivityState } from "./activity";
import type { ProcessState } from "./model";

interface ActivityMotion {
  readonly period: number | null;
  readonly scale?: number;
  /** Translations in the 512-unit face space. */
  readonly sway?: number;
  readonly lift?: number;
  readonly tilt?: number;
  readonly ringRange: readonly [min: number, max: number];
}

const ACTIVITY_MOTION = {
  thinking: {
    period: 3,
    scale: 0.04,
    ringRange: [0.35, 0.66],
  },
  working: {
    period: 1.2,
    scale: 0.056,
    ringRange: [0.38, 0.84],
  },
  drafting: {
    period: 2,
    sway: 14,
    ringRange: [0.32, 0.73],
  },
  waiting: {
    period: 6,
    ringRange: [0.28, 0.72],
  },
  approval: {
    period: 2,
    lift: 15,
    ringRange: [0.86, 0.86],
  },
  retry: { period: 3, tilt: 4, ringRange: [0.32, 0.76] },
  stopping: {
    period: null,
    ringRange: [0.36, 0.36],
  },
} as const satisfies Record<RosterActivityState, ActivityMotion>;

export function quietActivityMotion(
  state: ProcessState,
): ActivityMotion | null {
  return Object.hasOwn(ACTIVITY_MOTION, state)
    ? ACTIVITY_MOTION[state as RosterActivityState]
    : null;
}

export function sampleActivityMotion(motion: ActivityMotion, seconds: number) {
  "worklet";
  const angle =
    motion.period === null ? 0 : (seconds / motion.period) * 2 * Math.PI;
  const wave = Math.sin(angle);
  const pulse = (1 - Math.cos(angle)) / 2;
  const [min, max] = motion.ringRange;
  return {
    transform: {
      x: (motion.sway ?? 0) * wave,
      y: -(motion.lift ?? 0) * pulse,
      rotate: (motion.tilt ?? 0) * wave,
      scale: 1 + (motion.scale ?? 0) * pulse,
    },
    ringOpacity: min + (max - min) * pulse,
  };
}

/** Bounded vector analogues sampled on the native UI thread. */
export function quietEyeFrame(
  phase: number,
  enabled: boolean,
): "open" | "closed" | "left" | "right" {
  "worklet";
  if (!enabled) return "open";
  if (phase > 0.92 && phase < 0.945) return "closed";
  if (phase > 0.34 && phase < 0.45) return "left";
  if (phase > 0.66 && phase < 0.74) return "right";
  return "open";
}
export function quietMotion(
  effect: string,
  phase: number,
  level: number,
  pixels: number,
  moving: boolean,
  oneShot: boolean,
) {
  "worklet";
  const pulse = 0.5 + 0.5 * Math.sin(phase * Math.PI * 2),
    volume = Math.max(0, Math.min(1, level));
  const speaking = effect === "output-pulse";
  const y =
    moving && effect === "happy"
      ? -Math.sin(phase * Math.PI) * pixels * 0.04
      : 0;
  const scale = !moving
    ? 1
    : speaking
      ? 1 + 0.04 * volume + 0.005 * pulse
      : effect === "blink"
        ? 1 + 0.018 * pulse
        : oneShot
          ? 1 + 0.025 * Math.sin(phase * Math.PI)
          : 1 + 0.045 * pulse;
  return {
    y,
    scale,
    haloOpacity: !moving
      ? 0.55
      : speaking
        ? 0.4 + 0.6 * volume
        : 0.3 + 0.35 * (oneShot ? Math.sin(phase * Math.PI) : pulse),
    haloScale: !moving ? 1 : speaking ? 1 + 0.06 * volume : 1 + 0.025 * pulse,
    haloRotate: moving && effect === "working-accent" ? phase * 360 : 0,
  };
}
