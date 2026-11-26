import artwork from "./rive/prop-art.json";
import {
  processState,
  type AvatarMood,
  type AvatarConfig,
  type AvatarTheme,
} from "./model";
import { FACE_GEOMETRY, mix, contrastRatio } from "./draw";

export interface PropTransform {
  x: number;
  y: number;
  rotation: number;
  scaleX: number;
  scaleY: number;
}
export interface PropShape {
  type: "rect" | "circle" | "polygon" | "polyline";
  x?: number;
  y?: number;
  width?: number;
  height?: number;
  rx?: number;
  cx?: number;
  cy?: number;
  r?: number;
  points?: readonly (readonly [number, number])[];
  fill: string;
  stroke?: string;
  strokeWidth?: number;
}
export interface PropNode extends PropTransform {
  name: string;
  children: readonly PropNode[];
  shapes: readonly PropShape[];
}
export const PROP_ART = artwork as unknown as {
  viewBox: number;
  roots: readonly PropNode[];
};
export const PROP_NODES: readonly PropNode[] = PROP_ART.roots.flatMap(
  function walk(node): PropNode[] {
    return [node, ...node.children.flatMap(walk)];
  },
);
export type PropPose = Readonly<Record<string, PropTransform>>;

/** Materials come from the avatar's identity palette, including deep/pastel sets. */
export function propMaterial(
  config: AvatarConfig,
  theme: AvatarTheme,
  surface = config.choices.surface ?? "flat",
) {
  const [light = "#b48cff", deep = "#5e00f5"] =
    FACE_GEOMETRY.palettes[config.choices.palette ?? "core-nyx"]?.colors ?? [];
  const base = mix(light, deep, 0.3);
  const ink =
    contrastRatio(base, "#15121d") >= contrastRatio(base, "#faf8ff")
      ? "#15121d"
      : "#faf8ff";
  return {
    base,
    highlight: mix(light, "#ffffff", 0.22),
    low: mix(deep, "#15121d", 0.22),
    ink,
    edge: mix(base, ink, 0.56),
    shadow: theme === "dark" ? "#09080d" : mix(deep, "#15121d", 0.45),
    surface,
  };
}
const stateProp: Partial<Record<ReturnType<typeof processState>, string>> = {
  thinking: "Thought",
  drafting: "Keyboard",
  working: "Tool",
  waiting: "Clock",
  approval: "Request",
  retry: "Retry",
  stopping: "Stop",
  failure: "Warning",
};
/** A representative still, including when motion is reduced or the rig fails. */
export function staticPropPose(mood: AvatarMood): PropPose {
  const active = stateProp[processState(mood)];
  return Object.fromEntries(
    PROP_NODES.map((node) => [
      node.name,
      {
        x: node.x,
        y: node.y,
        rotation: node.rotation,
        scaleX: PROP_ART.roots.includes(node)
          ? +(node.name === active)
          : node.scaleX,
        scaleY: PROP_ART.roots.includes(node)
          ? +(node.name === active)
          : node.scaleY,
      },
    ]),
  );
}
export function propTransform(pose: PropTransform): string {
  if (
    ![pose.x, pose.y, pose.rotation, pose.scaleX, pose.scaleY].every(
      Number.isFinite,
    )
  )
    return "scale(0)";
  const n = (value: number) => +value.toFixed(4);
  return `translate(${n(pose.x)} ${n(pose.y)}) rotate(${n((pose.rotation * 180) / Math.PI)}) scale(${n(pose.scaleX)} ${n(pose.scaleY)})`;
}
