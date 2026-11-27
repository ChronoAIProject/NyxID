import { processState, type AvatarMood } from "./model";
import type { DrawList } from "./draw";
import { FACE_GEOMETRY } from "./draw";
import type { PropPose } from "./props";

/** Stable values in the Rive AvatarEmote.emote data binding. */
export const EMOTES = [
  {
    id: "idle",
    label: "Resting",
    description: "A resting face with an occasional blink.",
  },
  {
    id: "thinking",
    label: "Thinking",
    description:
      "A thought bubble forms while the avatar considers the next step.",
  },
  {
    id: "drafting",
    label: "Typing",
    description:
      "A keyboard appears, with short bursts of tapping and key presses.",
  },
  {
    id: "working",
    label: "Using tools",
    description:
      "A wrench grips a bolt, turns it, and repositions for the next turn.",
  },
  {
    id: "waiting",
    label: "Waiting",
    description: "A small clock advances while the avatar waits.",
  },
  {
    id: "approval",
    label: "Needs approval",
    description:
      "A request card appears with an empty checkbox, waiting for your decision.",
  },
  {
    id: "retry",
    label: "Retrying",
    description: "A return arrow resets, then holds while the task retries.",
  },
  {
    id: "stopping",
    label: "Stopping",
    description:
      "A stop tile settles into place while the stop request completes.",
  },
  {
    id: "failure",
    label: "Error",
    description: "A warning tile appears with a brief reaction, then holds.",
  },
  {
    id: "listening",
    label: "Listening",
    description: "An open expression while audio input is accepted.",
  },
  {
    id: "speaking",
    label: "Speaking",
    description: "A relaxed expression during audio playback.",
  },
  {
    id: "success",
    label: "Reply complete",
    description: "The keyboard retracts and the avatar returns to rest.",
  },
] as const;
export type Emote = (typeof EMOTES)[number]["id"];
export interface EyePose {
  x: number;
  y: number;
  openness: number;
}
export interface EmotePose extends EyePose {
  props?: PropPose;
}
export const RESTING_EYES: EyePose = { x: 0, y: 0, openness: 1 };
/** Edge-defined character paint cannot fill a vacated aperture coherently. */
export function supportsNaturalEyes(surface: string): boolean {
  return surface !== "ascii-outline" && surface !== "ascii-inverted";
}
export function emoteIndex(mood: AvatarMood): number {
  const state = processState(mood);
  if (state === "created") return 11;
  const index = EMOTES.findIndex((entry) => entry.id === state);
  return index < 0
    ? ["connecting", "reconnecting"].includes(state)
      ? 4
      : 0
    : index;
}

/** Move only the aperture geometry; tile, head, paint and texture stay fixed. */
export function eyePoseMasks(
  scene: DrawList,
  head: string,
  eyes: string,
  pose: EyePose,
) {
  if (
    ![pose.x, pose.y, pose.openness].every(Number.isFinite) ||
    (Math.abs(pose.x) < 0.00001 &&
      Math.abs(pose.y) < 0.00001 &&
      Math.abs(pose.openness - 1) < 0.00001)
  )
    return { mask: scene.mask, outsideMask: scene.outsideMask };
  const geometry =
    FACE_GEOMETRY.heads[head === "logo" ? "nyxbot" : head] ??
    FACE_GEOMETRY.heads.round!;
  const eye = geometry.eyes[eyes] ?? geometry.eyes.tall!;
  const points = eye.contours.flat();
  const cy = points.reduce((sum, point) => sum + point[1], 0) / points.length;
  const units = (geometry.viewBox[2] ?? 24) / 24;
  const dx = Math.max(-0.7, Math.min(0.7, pose.x)) * units;
  const dy = Math.max(-0.7, Math.min(0.7, pose.y)) * units;
  const sy = Math.max(0.12, Math.min(1.15, pose.openness));
  const n = (value: number) => +value.toFixed(4);
  const contours = eye.contours.map((path) =>
    path.map(([x, y]) => [x + dx, cy + (y - cy) * sy + dy] as const),
  );
  const paths = (
    polygons: readonly (readonly (readonly [number, number])[])[],
  ) =>
    polygons
      .map((path) => `M${path.map(([x, y]) => `${n(x)} ${n(y)}`).join("L")}Z`)
      .join("");
  const [tx, ty, scale] = scene.transform
    .match(/[-+]?\d*\.?\d+/g)!
    .map(Number) as [number, number, number];
  return {
    mask: scene.headMask + paths(contours),
    outsideMask:
      "M0 0H512V512H0Z" +
      paths(
        [...geometry.contours, ...contours].map((path) =>
          path.map(([x, y]) => [x * scale + tx, y * scale + ty] as const),
        ),
      ),
  };
}
