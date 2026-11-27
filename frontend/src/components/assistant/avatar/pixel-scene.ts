import {
  FACE_GEOMETRY,
  mix,
  type DrawList,
} from "../../../../../shared/agent-avatars/draw";
import {
  eyePoseMasks,
  RESTING_EYES,
  type EmotePose,
} from "../../../../../shared/agent-avatars/emotes";
import {
  PROP_ART,
  propMaterial,
  staticPropPose,
  type PropNode,
  type PropShape,
} from "../../../../../shared/agent-avatars/props";
import type {
  AvatarConfig,
  AvatarMood,
  AvatarTheme,
} from "@/lib/assistant/agent-avatar-model";
import { paintAvatar } from "./canvas";

/** Integer display multiples at the actual 192/112/56px review sizes. */
export function pixelGrid(pixels: number): number {
  return Math.max(
    16,
    Math.min(
      64,
      Math.round(pixels / (pixels >= 128 ? 3 : pixels >= 96 ? 2 : 1)),
    ),
  );
}
const rgb = (colour: string) =>
  [1, 3, 5].map((i) => parseInt(colour.slice(i, i + 2), 16));
const colourCaches = new WeakMap<DrawList, Map<number, number[]>>();
function shapePath(context: CanvasRenderingContext2D, shape: PropShape) {
  context.beginPath();
  if (shape.type === "rect")
    context.roundRect(
      shape.x!,
      shape.y!,
      shape.width!,
      shape.height!,
      shape.rx!,
    );
  else if (shape.type === "circle")
    context.arc(shape.cx!, shape.cy!, shape.r!, 0, Math.PI * 2);
  else {
    shape.points?.forEach(([x, y], index) =>
      index ? context.lineTo(x, y) : context.moveTo(x, y),
    );
    if (shape.type === "polygon") context.closePath();
  }
}

/** Paint face and props together before quantization, so every edge shares one grid. */
export function paintPixelScene(
  context: CanvasRenderingContext2D,
  {
    scene,
    materialScene,
    config,
    mood,
    pose = RESTING_EYES,
    theme,
    pixels,
    eyes = true,
    props = true,
    quiet = false,
  }: {
    scene: DrawList;
    materialScene?: DrawList;
    config: AvatarConfig;
    mood: AvatarMood;
    pose?: EmotePose;
    theme: AvatarTheme;
    pixels: number;
    eyes?: boolean;
    props?: boolean;
    quiet?: boolean;
  },
) {
  const grid = context.canvas.width;
  const snap = (value: number) => (Math.round((value * grid) / 24) * 24) / grid;
  const masks = eyePoseMasks(
    scene,
    config.choices.head ?? "round",
    config.choices.eyes ?? "tall",
    eyes ? { ...pose, x: snap(pose.x), y: snap(pose.y) } : RESTING_EYES,
  );
  const source = materialScene ?? scene;
  paintAvatar(
    context,
    {
      ...source,
      mask: masks.mask,
      outsideMask: masks.outsideMask,
      paths: source.paths.map((path) =>
        path.source && path.d === source.mask
          ? { ...path, d: masks.mask }
          : path,
      ),
    },
    grid,
  );
  const material = propMaterial(config, theme, "flat");
  if (props) {
    const transforms = quiet
      ? staticPropPose(mood)
      : (pose.props ?? staticPropPose(mood));
    const colour = (value: string, part: string) =>
      ({
        "#f5f0ff": material.base,
        "#d7c5ff": material.low,
        "#302644": material.ink,
        "#9872ee": part.startsWith("Key") ? material.highlight : material.ink,
        "#f7c76a": material.base,
        "#bbc7da": material.highlight,
        "#171320": material.shadow,
      })[value] ?? value;
    const draw = (node: PropNode) => {
      const transform = transforms[node.name];
      if (
        !transform ||
        ![
          transform.x,
          transform.y,
          transform.rotation,
          transform.scaleX,
          transform.scaleY,
        ].every(Number.isFinite) ||
        Math.abs(transform.scaleX) < 0.01 ||
        Math.abs(transform.scaleY) < 0.01
      )
        return;
      context.save();
      // Round translations to the same pixel lattice as the avatar.
      context.translate(
        (Math.round((transform.x * grid) / 240) * 240) / grid,
        (Math.round((transform.y * grid) / 240) * 240) / grid,
      );
      context.rotate(transform.rotation);
      context.scale(transform.scaleX, transform.scaleY);
      if (
        pixels >= 96 ||
        !["TapLeft", "TapRight", "ClockPin"].includes(node.name)
      )
        for (const shape of node.shapes) {
          shapePath(context, shape);
          if (shape.fill !== "none") {
            context.fillStyle = colour(shape.fill, node.name);
            context.fill();
          }
          if (shape.stroke) {
            context.strokeStyle = colour(shape.stroke, node.name);
            context.lineWidth = Math.max(shape.strokeWidth ?? 1, 240 / grid);
            context.lineJoin = "miter";
            context.lineCap = "butt";
            context.stroke();
          }
        }
      node.children.forEach(draw);
      context.restore();
    };
    context.save();
    context.scale(grid / PROP_ART.viewBox, grid / PROP_ART.viewBox);
    PROP_ART.roots.forEach(draw);
    context.restore();
  }
  // One small palette, hard alpha, and nearest-neighbour display remove smooth/vector edges.
  const [light = "#b48cff", deep = "#5e00f5"] =
    FACE_GEOMETRY.palettes[config.choices.palette ?? "core-nyx"]?.colors ?? [];
  const palette = [
    ...new Set([
      light,
      mix(light, deep, 1 / 3),
      mix(light, deep, 2 / 3),
      deep,
      material.ink,
      material.low,
      "#0c0c0e",
      "#ffffff",
    ]),
  ].map(rgb);
  const bitmap = context.getImageData(0, 0, grid, grid);
  let cache = colourCaches.get(scene);
  if (!cache) {
    cache = new Map<number, number[]>();
    colourCaches.set(scene, cache);
  }
  for (let i = 0; i < bitmap.data.length; i += 4) {
    if (bitmap.data[i + 3]! < 128) {
      bitmap.data[i + 3] = 0;
      continue;
    }
    const r = bitmap.data[i]!,
      g = bitmap.data[i + 1]!,
      b = bitmap.data[i + 2]!;
    const key = r * 65536 + g * 256 + b;
    let nearest = cache.get(key);
    if (!nearest) {
      let best = Infinity;
      for (const candidate of palette) {
        const distance =
          (r - candidate[0]!) ** 2 +
          (g - candidate[1]!) ** 2 +
          (b - candidate[2]!) ** 2;
        if (distance < best) {
          best = distance;
          nearest = candidate;
        }
      }
      if (cache.size >= 4096) cache.clear();
      cache.set(key, nearest!);
    }
    bitmap.data[i] = nearest![0]!;
    bitmap.data[i + 1] = nearest![1]!;
    bitmap.data[i + 2] = nearest![2]!;
    bitmap.data[i + 3] = 255;
  }
  context.putImageData(bitmap, 0, 0);
}
