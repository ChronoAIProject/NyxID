import type { PixelPlane } from "./png";
/** Platform-neutral draw lists in a fixed 512px export space.
 * The only shape sources are generated reference paths/contours. Background
 * fields and quantizers use the reference equations; no DOM, fonts or Path2D.
 */
import geometryJson from "./geometry.json";
import {
  defaultAvatar,
  processState,
  type AvatarConfig,
  type AvatarMood,
  type AvatarTheme,
} from "./model";
import { reactionFor } from "./process";
import { sampleActivityMotion } from "./quiet-motion";
type Point = readonly [number, number];
interface Eye {
  d: string;
  contours: Point[][];
}
interface Head extends Eye {
  viewBox: readonly number[];
  eyes: Record<string, Eye>;
}
interface Palette {
  colors: string[];
}
interface Ripple {
  a: string;
  b: string;
  bg: string;
  ring: string;
  face: string;
  light?: boolean;
  text: string;
}
export const FACE_GEOMETRY = geometryJson as unknown as {
  heads: Record<string, Head>;
  tints: { fg: string; solid: string }[];
  palettes: Record<string, Palette>;
  ripples: Record<string, Ripple>;
  bayer: number[][];
  effects: {
    dither_grids: number[];
    ascii_columns: number[];
    ascii_ramp: string;
    churn_chars: string;
  };
};
export interface DrawPath {
  readonly d: string;
  readonly fill: string;
  readonly role?: "activity-ring";
  readonly opacity?: number;
  readonly clip?: "face" | "outside";
  readonly stroke?: string;
  readonly width?: number;
  readonly source?: boolean;
  readonly crisp?: boolean;
}
export interface DrawList {
  readonly version: 1;
  readonly mask: string;
  readonly headMask: string;
  readonly outsideMask: string;
  readonly transform: string;
  readonly frame: "circle" | "tile" | "none";
  readonly paths: readonly DrawPath[];
  /** Painted after backgroundPlane, before the masked foreground plane. */
  readonly backgroundPaths: readonly DrawPath[];
  readonly backgroundPlane?: PixelPlane;
  readonly foregroundPlane?: PixelPlane;
  readonly state: string;
  readonly surface: string;
  readonly contrastAdjusted?: boolean;
  readonly fallback?: "size" | "eye-grid";
  readonly reaction: string;
  readonly motion: { x: number; y: number; rotate: number; scale: number };
}
export interface DrawOptions {
  size?: number;
  ambient?: string;
  kind?: string;
  theme?: AvatarTheme;
  mood?: AvatarMood;
  seconds?: number;
  reduced?: boolean;
  native?: boolean;
  transparent?: boolean;
  resting?: boolean;
  eyeFrame?: "open" | "closed" | "left" | "right";
  /** Internal material under moving eye apertures; never used by saved/exported stills. */
  materialEyes?: boolean;
  level?: number;
}
const n = (v: number) => +v.toFixed(4);
const clamp = (v: number) => Math.max(0, Math.min(1, v));
export const noise = (a: number, b: number, c = 0) => {
  let h = (a * 374761393 + b * 668265263 + c * 1442695041) | 0;
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  return ((h ^ (h >>> 16)) >>> 0) / 4294967296;
};
export const lightAt = (u: number, v: number) => 1 - (0.75 * (u + v)) / 2;
export const bayer = (i: number, j: number) =>
  (FACE_GEOMETRY.bayer[j & 7]![i & 7]! + 0.5) / 64;
const rgb = (hex: string) =>
  [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
export function mix(a: string, b: string, t: number): string {
  const A = rgb(a),
    B = rgb(b);
  return (
    "#" +
    A.map((v, i) =>
      Math.round(v + (B[i]! - v) * clamp(t))
        .toString(16)
        .padStart(2, "0"),
    ).join("")
  );
}
export function contrastRatio(a: string, b: string): number {
  const luminance = (hex: string) =>
    rgb(hex)
      .map((c) => {
        const v = c / 255;
        return v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4;
      })
      .reduce((sum, c, i) => sum + c * [0.2126, 0.7152, 0.0722][i]!, 0);
  const x = luminance(a),
    y = luminance(b);
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
const rect = (x: number, y: number, w: number, h: number) =>
  `M${n(x)} ${n(y)}h${n(w)}v${n(h)}h${n(-w)}Z`;
const dot = (x: number, y: number, r: number) =>
  `M${n(x - r)} ${n(y)}a${n(r)} ${n(r)} 0 1 0 ${n(2 * r)} 0a${n(r)} ${n(r)} 0 1 0 ${n(-2 * r)} 0Z`;
function polygonPaths(
  paths: readonly Point[][],
  dx = 0,
  dy = 0,
  sy = 1,
  cy = 0,
) {
  return paths
    .map(
      (p) =>
        p
          .map(
            ([x, y], i) =>
              `${i ? "L" : "M"}${n(x + dx)} ${n(cy + (y - cy) * sy + dy)}`,
          )
          .join("") + "Z",
    )
    .join("");
}
interface Mask {
  sample: (u: number, v: number) => boolean;
  head: (u: number, v: number) => boolean;
}
const MASK_LIMIT = 48,
  MASK_SIZE = 256;
const masks = new Map<string, Mask>();
function rasterContours(
  paths: readonly Point[][],
  vb: readonly number[],
  pad: number,
) {
  const [vx = 0, vy = 0, vs = 24] = vb,
    scale = (MASK_SIZE * (1 - 2 * pad)) / vs,
    offset = MASK_SIZE * pad;
  const out = new Uint8Array(MASK_SIZE * MASK_SIZE);
  for (let row = 0; row < MASK_SIZE; row++) {
    const y = (row + 0.5 - offset) / scale + vy,
      xs: number[] = [];
    for (const path of paths)
      for (let i = 0, j = path.length - 1; i < path.length; j = i++) {
        const a = path[j]!,
          b = path[i]!;
        if (a[1] > y !== b[1] > y)
          xs.push(
            (a[0] + ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]) - vx) * scale +
              offset,
          );
      }
    xs.sort((a, b) => a - b);
    for (let i = 0; i + 1 < xs.length; i += 2) {
      const left = Math.max(0, Math.ceil(xs[i]! - 0.5)),
        right = Math.min(MASK_SIZE, Math.ceil(xs[i + 1]! - 0.5));
      out.fill(1, row * MASK_SIZE + left, row * MASK_SIZE + right);
    }
  }
  return (u: number, v: number) =>
    u >= 0 &&
    v >= 0 &&
    u < 1 &&
    v < 1 &&
    out[Math.floor(v * MASK_SIZE) * MASK_SIZE + Math.floor(u * MASK_SIZE)] ===
      1;
}
export function faceMask(headId: string, eyesId: string, pad = 0): Mask {
  const key = `${headId}:${eyesId}:${pad}`,
    cached = masks.get(key);
  if (cached) {
    masks.delete(key);
    masks.set(key, cached);
    return cached;
  }
  const h = FACE_GEOMETRY.heads[headId]!,
    eye = h.eyes[eyesId]!;
  const value = {
    sample: rasterContours([...h.contours, ...eye.contours], h.viewBox, pad),
    head: rasterContours(h.contours, h.viewBox, pad),
  };
  if (masks.size >= MASK_LIMIT) masks.delete(masks.keys().next().value!);
  masks.set(key, value);
  return value;
}
const topologyCache = new Map<string, number>();
/** Count enclosed empty regions at the exact selected texture grid. */
export function coarseEyeHoles(
  head: string,
  eyes: string,
  columns: number,
  rows = columns,
  pad = 0.12,
): number {
  const key = `${head}:${eyes}:${columns}:${rows}:${pad}`,
    cached = topologyCache.get(key);
  if (cached !== undefined) return cached;
  const { sample } = faceMask(head, eyes, pad),
    seen = new Uint8Array(columns * rows);
  let holes = 0;
  for (let y = 0; y < rows; y++)
    for (let x = 0; x < columns; x++) {
      const start = y * columns + x;
      if (seen[start] || sample((x + 0.5) / columns, (y + 0.5) / rows))
        continue;
      const todo = [start];
      seen[start] = 1;
      let exterior = false;
      while (todo.length) {
        const at = todo.pop()!,
          px = at % columns,
          py = Math.floor(at / columns);
        if (px === 0 || py === 0 || px === columns - 1 || py === rows - 1)
          exterior = true;
        for (const [dx = 0, dy = 0] of [
          [-1, 0],
          [1, 0],
          [0, -1],
          [0, 1],
        ]) {
          const nx = px + dx,
            ny = py + dy;
          if (nx < 0 || ny < 0 || nx >= columns || ny >= rows) continue;
          const next = ny * columns + nx;
          if (!seen[next] && !sample((nx + 0.5) / columns, (ny + 0.5) / rows)) {
            seen[next] = 1;
            todo.push(next);
          }
        }
      }
      if (!exterior) holes++;
    }
  if (topologyCache.size >= 256)
    topologyCache.delete(topologyCache.keys().next().value!);
  topologyCache.set(key, holes);
  return holes;
}
export function textureGrid(
  surface: string,
): { columns: number; rows: number } | null {
  if (surface.startsWith("dither-")) {
    const index = [
        "ordered",
        "chunky",
        "fine",
        "diffusion",
        "halftone",
        "phosphor",
        "handheld",
        "grain",
        "duotone",
      ].indexOf(surface.slice(7)),
      columns = FACE_GEOMETRY.effects.dither_grids[index] ?? 48;
    return { columns, rows: columns };
  }
  if (surface.startsWith("ascii-")) {
    const index = [
        "shaded",
        "blocks",
        "braille",
        "binary",
        "name",
        "outline",
        "gradient",
        "hatching",
        "inverted",
      ].indexOf(surface.slice(6)),
      columns = FACE_GEOMETRY.effects.ascii_columns[index] ?? 30,
      rows = Math.floor(columns * 0.6);
    return surface === "ascii-braille"
      ? { columns: columns * 2, rows: rows * 4 }
      : { columns, rows };
  }
  return null;
}
export function resolveFaceSurface(
  surface: string,
  head: string,
  eyes: string,
  _size: number,
): { surface: string; fallback?: "size" | "eye-grid" } {
  // Texture geometry is authored in the same 512-unit frame at every display size.
  const grid = textureGrid(surface),
    pad = surface.startsWith("ascii-") ? 0.02 : head === "nyxbot" ? 0.14 : 0.12;
  if (
    grid &&
    coarseEyeHoles(head, eyes, grid.columns, grid.rows, pad) !==
      (eyes === "visor" ? 1 : 2)
  )
    return { surface: "flat", fallback: "eye-grid" };
  return { surface };
}
export function maskCacheInfo() {
  return {
    entries: masks.size,
    maxEntries: MASK_LIMIT,
    maxBytes: MASK_LIMIT * 2 * MASK_SIZE * MASK_SIZE,
  };
}
/** Finite 5x7 vector font. Decorative pixels; never accessibility text. */
const FONT: Record<string, string[]> = {
  "0": ["01110", "10001", "10011", "10101", "11001", "10001", "01110"],
  "1": ["00100", "01100", "00100", "00100", "00100", "00100", "01110"],
  ".": ["00000", "00000", "00000", "00000", "00000", "00110", "00110"],
  ":": ["00000", "00110", "00110", "00000", "00110", "00110", "00000"],
  "-": ["00000", "00000", "00000", "11111", "00000", "00000", "00000"],
  "=": ["00000", "00000", "11111", "00000", "11111", "00000", "00000"],
  "+": ["00000", "00100", "00100", "11111", "00100", "00100", "00000"],
  "*": ["00000", "10101", "01110", "11111", "01110", "10101", "00000"],
  "#": ["01010", "01010", "11111", "01010", "11111", "01010", "01010"],
  "%": ["11001", "11010", "00100", "01000", "10110", "00110", "00000"],
  "@": ["01110", "10001", "10111", "10101", "10111", "10000", "01110"],
  "/": ["00001", "00010", "00010", "00100", "01000", "01000", "10000"],
  X: ["10001", "10001", "01010", "00100", "01010", "10001", "10001"],
  n: ["00000", "00000", "11110", "10001", "10001", "10001", "10001"],
  y: ["00000", "00000", "10001", "10001", "01111", "00001", "01110"],
  x: ["00000", "00000", "10001", "01010", "00100", "01010", "10001"],
  i: ["00100", "00000", "01100", "00100", "00100", "00100", "01110"],
  d: ["00001", "00001", "01111", "10001", "10001", "10001", "01111"],
  "·": ["00000", "00000", "00000", "00100", "00000", "00000", "00000"],
};
function glyph(
  char: string,
  x: number,
  y: number,
  w: number,
  h: number,
): string {
  if (char === "█") return rect(x, y, w, h);
  if ("░▒▓".includes(char)) {
    let d = "";
    const density = "░▒▓".indexOf(char) + 1;
    for (let j = 0; j < 7; j++)
      for (let i = 0; i < 5; i++)
        if ((i + j * 3) % 4 < density)
          d += rect(
            x + (w * i) / 5,
            y + (h * j) / 7,
            (w / 5) * 0.8,
            (h / 7) * 0.8,
          );
    return d;
  }
  return (FONT[char] ?? [])
    .map((row, j) =>
      [...row]
        .map((v, i) =>
          v === "1" ? rect(x + (w * i) / 5, y + (h * j) / 7, w / 5, h / 7) : "",
        )
        .join(""),
    )
    .join("");
}
function backgroundColor(
  type: string,
  u: number,
  v: number,
  a: string,
  b: string,
  ink: string,
): string | null {
  if (type === "transparent") return null;
  if (type === "solid") return ink === "#ffffff" ? a : b;
  if (type === "dark") return "#0c0c0e";
  if (type === "tint") return b;
  if (type === "pastel") return mix("#ffffff", a, 0.32);
  if (type === "gradient") return mix(a, b, (u + v) / 2);
  if (type === "three-stop") {
    const t = clamp((u * 0.342 + v * 0.94) / 1.282);
    return mix(b, a, 1 - Math.abs(t * 2 - 1));
  }
  if (type === "conic") {
    const t =
      ((Math.atan2(v - 0.5, u - 0.5) - (200 * Math.PI) / 180) / (2 * Math.PI) +
        2) %
      1;
    return mix(a, b, 1 - Math.abs(2 * t - 1));
  }
  if (type === "mesh") {
    const t1 = clamp(1 - Math.hypot((u - 0.18) / 0.7, (v - 0.12) / 0.7) / 0.7),
      t2 = clamp(1 - Math.hypot((u - 0.9) / 0.8, (v - 0.92) / 0.8) / 0.75);
    return mix(mix(mix(a, b, 0.55), b, t2), a, t1);
  }
  if (type === "glow") {
    const d = Math.hypot((u - 0.5) / 1.2, (v - 1.1) / 0.9);
    return mix(b, "#0b0a16", d / 0.7);
  }
  if (type === "glass") {
    let c = mix(a, b, clamp((u * 0.342 + v * 0.94) / 1.282));
    c = mix(c, "#ffffff", 0.42 * clamp(1 - v / 0.46));
    return mix(c, "#000000", 0.18 * clamp((v - 0.7) / 0.3));
  }
  return "#0c0c0e";
}
const stills = new Map<string, DrawList>();
const STILL_LIMIT = 16,
  STILL_BYTES = 8 * 1024 * 1024;
let stillBytes = 0;
const drawBytes = (list: DrawList) =>
  ((list.backgroundPlane?.rgba.byteLength ?? 0) +
    (list.foregroundPlane?.rgba.byteLength ?? 0)) *
    4 +
  2 *
    (list.mask.length +
      list.outsideMask.length +
      [...list.backgroundPaths, ...list.paths].reduce(
        (sum, p) => sum + p.d.length + 32,
        0,
      ));
function faceMotion(
  effect: string,
  t: number,
  level: number | undefined,
  state: string,
  animated: boolean,
) {
  const pulse = animated ? 0.5 + 0.5 * Math.sin(t * 2.4) : 0.5;
  return {
    x: 0,
    y:
      animated && effect === "hop"
        ? -Math.sin(Math.min(1, t / 1.2) * Math.PI) * 16
        : 0,
    rotate:
      animated && effect === "settling"
        ? Math.sin(t * 2) * Math.max(0, 1 - t) * 2
        : 0,
    scale:
      animated &&
      [
        "pulse",
        "blink",
        "slow-pulse",
        "output-pulse",
        "thinking-pulse",
        "working-accent",
        "drafting-pulse",
        "listening-ring",
        "assembly-pulse",
        "connected-accent",
      ].includes(effect)
        ? 1 +
          (effect === "blink" ? 0.018 : 0.045) *
            pulse *
            (state === "speaking" ? clamp(level ?? 0.4) : 1)
        : 1,
  };
}
export function drawAvatar(
  config: AvatarConfig,
  options: DrawOptions = {},
): DrawList {
  const size = options.size ?? 112,
    mood = options.mood ?? "idle",
    t = options.seconds ?? 0,
    state = processState(mood);
  const c = {
    ...defaultAvatar("", config.choices.head ? "specialist" : "nyxbot").choices,
    ...config.choices,
  };
  const reaction = reactionFor(
    config,
    mood,
    size,
    options.native,
    options.reduced,
  );
  const animated =
    t > 0 &&
    size >= 24 &&
    !reaction.still &&
    !options.resting &&
    reaction.activityMotion?.period !== null;
  // Quiet live states use recognizable motion in place of the saved
  // analogue. The saved Still preference and rich reactions take precedence.
  const activityMotion =
    reaction.activityMotion && !options.resting
      ? sampleActivityMotion(reaction.activityMotion, t)
      : null;
  const pulse = animated ? 0.5 + 0.5 * Math.sin(t * 2.4) : 0.5;
  const ringOpacity = activityMotion?.ringOpacity ?? 0.35 + 0.3 * pulse;
  // Quiet reactions move the cached textured still. They never rebuild a
  // character/dither field on every frame or switch the saved surface to Flat.
  if (animated && reaction.quiet && c.surface !== "flat") {
    const still = drawAvatar(config, { ...options, seconds: 0 });
    if (still.surface !== "flat") {
      const paths =
        state !== "idle" && reaction.id !== "still"
          ? still.paths.map((path, index) =>
              (
                activityMotion
                  ? path.role === "activity-ring"
                  : index === still.paths.length - 1
              )
                ? { ...path, opacity: ringOpacity }
                : path,
            )
          : still.paths;
      return {
        ...still,
        paths,
        reaction: reaction.effective,
        motion:
          activityMotion?.transform ??
          faceMotion(reaction.effective, t, options.level, state, true),
      };
    }
  }
  const key = JSON.stringify([
    c,
    size < 24 ? 16 : size < 64 ? 44 : size < 96 ? 80 : 112,
    options.theme,
    options.ambient,
    options.kind,
    state,
    options.reduced,
    options.native,
    options.transparent,
    options.resting,
    options.eyeFrame,
    options.materialEyes,
  ]);
  if (!animated && stills.has(key)) {
    const value = stills.get(key)!;
    stills.delete(key);
    stills.set(key, value);
    return value;
  }
  const effect = animated ? reaction.effective : "still";
  const shuffle = animated && effect === "face-shuffle" && t < 1.4;
  const hId =
      shuffle && c.head
        ? ["block", "tall", "wide", "round"][Math.floor(t / 0.24) % 4]!
        : c.head === "logo"
          ? "nyxbot"
          : (c.head ?? "nyxbot"),
    h = FACE_GEOMETRY.heads[hId] ?? FACE_GEOMETRY.heads.nyxbot!,
    eyes = shuffle
      ? ["dots", "tall", "calm", "happy", "wink", "visor"][
          Math.floor(t / 0.24) % 6
        ]!
      : (c.eyes ?? "dots");
  const [a = "#b48cff", b = "#5e00f5", ink = "#ffffff"] =
    FACE_GEOMETRY.palettes[c.palette ?? "core-nyx"]?.colors ?? [];
  let surface = c.surface ?? "flat";
  if (
    size >= 96 &&
    !options.native &&
    animated &&
    ["character-churn", "rain", "typing", "scan"].includes(effect)
  )
    surface = "ascii-shaded";
  if (
    size >= 96 &&
    !options.native &&
    animated &&
    ["moving-light", "pixel-assembly"].includes(effect)
  )
    surface = "dither-ordered";
  const posedEyes =
    !options.resting && ["happy", "calm", "tall"].includes(reaction.static)
      ? reaction.static
      : eyes;
  const resolvedSurface = resolveFaceSurface(surface, hId, posedEyes, size);
  surface = resolvedSurface.surface;
  const coreIndex = [
    "core-violet",
    "core-ocean",
    "core-mint",
    "core-ember",
    "core-indigo",
    "core-graphite",
  ].indexOf(c.palette ?? "");
  const coreTint =
    c.palette === "core-nyx"
      ? FACE_GEOMETRY.tints[0]
      : FACE_GEOMETRY.tints[coreIndex];
  const identityTint =
    c.treatment === "tint" &&
    (c.background === "treatment" || c.background === "tint") &&
    !!coreTint &&
    options.kind === "specialist";
  const lightIdentity = identityTint && options.theme === "light" && !!c.head;
  const ascii = surface.startsWith("ascii-"),
    dither = surface.startsWith("dither-"),
    pad = ascii
      ? 0.02
      : dither
        ? hId === "nyxbot"
          ? 0.14
          : 0.12
        : identityTint
          ? hId === "nyxbot"
            ? 0.14
            : lightIdentity
              ? 0.115
              : 0.1
          : 0.15;
  const [vx = 0, vy = 0, vs = 24] = h.viewBox,
    scale = (512 * (1 - 2 * pad)) / vs,
    offset = 512 * pad;
  const transform = `translate(${n(offset - vx * scale)} ${n(offset - vy * scale)}) scale(${n(scale)})`;
  let eye = h.eyes[eyes] ?? h.eyes.dots!,
    eyeD = eye.d;
  // Rich native stills retain their exact sampled eyes; quiet whole-face motion is separate.
  const blinkFrame =
    animated &&
    !activityMotion &&
    reaction.id === "blink" &&
    t % 6 > 5.52 &&
    t % 6 < 5.67
      ? "closed"
      : animated &&
          !activityMotion &&
          reaction.id === "blink" &&
          t % 6 > 2 &&
          t % 6 < 2.7
        ? "left"
        : "open";
  const frame =
    surface === "flat" ? (options.eyeFrame ?? blinkFrame) : undefined;
  if (frame && frame !== "open") {
    const center =
      hId === "nyxbot"
        ? 86
        : eye.contours.flat().reduce((s, p) => s + p[1], 0) /
          eye.contours.flat().length;
    eyeD = polygonPaths(
      eye.contours,
      frame === "left" ? -vs * 0.018 : frame === "right" ? vs * 0.018 : 0,
      0,
      frame === "closed" ? 0.18 : 1,
      center,
    );
  }
  if (!options.resting && reaction.static === "happy")
    ((eye = h.eyes.happy!), (eyeD = eye.d));
  if (!options.resting && reaction.static === "calm")
    ((eye = h.eyes.calm!), (eyeD = eye.d));
  if (!options.resting && reaction.static === "tall")
    ((eye = h.eyes.tall!), (eyeD = eye.d));
  const mask = h.d + eyeD,
    originalSampler = faceMask(
      hId,
      eye === h.eyes[eyes]
        ? eyes
        : reaction.static === "happy"
          ? "happy"
          : reaction.static === "tall"
            ? "tall"
            : "calm",
      pad,
    ),
    maskSampler = options.materialEyes
      ? { ...originalSampler, sample: originalSampler.head }
      : originalSampler;
  let backgroundPlane: PixelPlane | undefined,
    foregroundPlane: PixelPlane | undefined;
  const groups = new Map<
    string,
    {
      d: string;
      fill: string;
      clip?: "face" | "outside";
      opacity?: number;
      crisp?: boolean;
    }
  >();
  function add(
    d: string,
    fill: string,
    clip?: "face" | "outside",
    opacity = 1,
  ) {
    if (!d) return;
    const crisp = d.includes("h") && !d.includes("a");
    const key = `${fill}:${clip ?? ""}:${opacity}:${crisp}`;
    const group = groups.get(key);
    if (group) group.d += d;
    else groups.set(key, { d, fill, clip, opacity, crisp });
  }
  const treatmentBackground: Record<string, string> = {
    tint: "tint",
    solid: "solid",
    "gradient-tile": "gradient",
    "three-stop": "three-stop",
    mesh: "mesh",
    "conic-sheen": "conic",
    glass: "glass",
    "gradient-face": "dark",
    "night-glow": "glow",
    pastel: "pastel",
    "neon-line": "dark",
  };
  const selectedBg = c.background ?? "treatment";
  const bg = options.transparent
    ? "transparent"
    : selectedBg === "treatment"
      ? treatmentBackground[c.treatment ?? "tint"]!
      : selectedBg;
  if (bg === "ripple") {
    const p = FACE_GEOMETRY.ripples[c.ripple_palette ?? "nyx"]!;
    add(rect(0, 0, 512, 512), p.bg);
    for (let j = 0; j < 32; j++)
      for (let i = 0; i < 32; i++) {
        const x = (i + 0.5) * 16,
          y = (j + 0.5) * 16,
          dist = Math.hypot(x - 256, y - 256) / 512,
          wave = 0.5 + 0.5 * Math.sin((animated ? t : 1.25) * 4 - dist * 14),
          fade = Math.max(0, 1 - dist * 0.8),
          r = 16 * 0.46 * (0.18 + 0.82 * wave) * (0.3 + 0.7 * fade);
        if (r >= 0.5)
          add(
            dot(x, y, r),
            mix(p.a, p.b, Math.round(wave * 32) / 32),
            undefined,
            n(0.25 + 0.75 * fade),
          );
      }
    add(dot(256, 256, 188), p.ring);
  } else if (
    ["gradient", "three-stop", "mesh", "conic", "glass", "glow"].includes(bg)
  ) {
    const rgba = new Uint8Array(48 * 48 * 4);
    for (let j = 0; j < 48; j++)
      for (let i = 0; i < 48; i++) {
        const color = rgb(
          backgroundColor(bg, (i + 0.5) / 48, (j + 0.5) / 48, a, b, ink)!,
        );
        const at = (j * 48 + i) * 4;
        rgba.set([...color, 255], at);
      }
    backgroundPlane = { width: 48, height: 48, rgba };
  } else {
    const color = identityTint
      ? lightIdentity
        ? coreTint!.solid
        : coreTint!.fg
      : backgroundColor(bg, 0.5, 0.5, a, b, ink);
    if (color)
      add(
        rect(0, 0, 512, 512),
        color,
        undefined,
        identityTint
          ? lightIdentity
            ? 1
            : hId === "nyxbot"
              ? 0.1
              : 0.15
          : bg === "tint"
            ? 0.15
            : 1,
      );
  }
  if (
    animated &&
    effect === "halftone-ripple" &&
    size >= 96 &&
    !options.native
  ) {
    const p = FACE_GEOMETRY.ripples[c.ripple_palette ?? "nyx"]!;
    for (let j = 0; j < 32; j++)
      for (let i = 0; i < 32; i++) {
        const u = (i + 0.5) / 32,
          v = (j + 0.5) / 32,
          dist = Math.hypot(u - 0.5, v - 0.5),
          wave = 0.5 + 0.5 * Math.sin(t * 4 - dist * 14);
        add(
          dot(u * 512, v * 512, 16 * 0.46 * (0.18 + 0.82 * wave)),
          mix(p.a, p.b, Math.round(wave * 32) / 32),
          undefined,
          0.65,
        );
      }
  }
  const backgroundPaths: DrawPath[] = [...groups.values()].filter(
    (p) => p.d.length > 0,
  );
  groups.clear();
  // Foreground and texture use exactly the same mask; eye holes always reveal the composition beneath.
  let faceColor = identityTint
    ? lightIdentity
      ? "#ffffff"
      : coreTint!.fg
    : bg === "ripple"
      ? FACE_GEOMETRY.ripples[c.ripple_palette ?? "nyx"]!.face
      : c.treatment === "solid"
        ? "#17171a"
        : c.treatment === "pastel"
          ? ink === "#ffffff"
            ? b
            : ink
          : c.treatment === "tint"
            ? ink === "#ffffff"
              ? a
              : b
            : c.treatment === "night-glow"
              ? a
              : ink;
  let materialA = a,
    materialB = b,
    contrastAdjusted = false;
  // A derived reference treatment keeps its authored colours. Explicit mixes
  // use existing palette inks when their chosen field would hide the face.
  if (bg !== treatmentBackground[c.treatment ?? "tint"]) {
    const backgrounds =
      bg === "transparent"
        ? [
            options.ambient ??
              (options.theme === "light" ? "#ffffff" : "#141416"),
          ]
        : bg === "ripple"
          ? [FACE_GEOMETRY.ripples[c.ripple_palette ?? "nyx"]!.ring]
          : [0.25, 0.5, 0.75].flatMap((u) =>
              [0.25, 0.5, 0.75].map(
                (v) => backgroundColor(bg, u, v, a, b, ink) ?? "#141416",
              ),
            );
    const score = (color: string) =>
      Math.min(...backgrounds.map((bg) => contrastRatio(color, bg)));
    const candidates = [faceColor, ink, a, b, "#ffffff", "#17171a"];
    const safe = candidates.reduce(
      (best, color) => (score(color) > score(best) ? color : best),
      faceColor,
    );
    if (c.treatment === "gradient-face" && Math.min(score(a), score(b)) < 2.5) {
      materialA = mix(safe, a, 0.1);
      materialB = mix(safe, b, 0.2);
      faceColor = safe;
      contrastAdjusted = true;
    } else if (score(faceColor) < 2.5 && score(safe) > score(faceColor)) {
      faceColor = safe;
      materialA = safe;
      materialB = mix(safe, b, 0.15);
      contrastAdjusted = true;
    }
  }
  const pairMaterial = ["gradient-face", "night-glow", "tint"].includes(
    c.treatment ?? "tint",
  );
  const textureHigh = pairMaterial ? materialA : faceColor;
  const textureLow = pairMaterial
    ? materialB
    : mix(faceColor, backgroundColor(bg, 0.5, 0.5, a, b, ink) ?? b, 0.35);
  const points = h.contours.flat(),
    minX = Math.min(...points.map((p) => p[0])),
    maxX = Math.max(...points.map((p) => p[0])),
    minY = Math.min(...points.map((p) => p[1])),
    maxY = Math.max(...points.map((p) => p[1]));
  const gradientAt = (u: number, v: number) => {
    const x = (((u - pad) * vs) / (1 - 2 * pad) + vx - minX) / (maxX - minX),
      y = (((v - pad) * vs) / (1 - 2 * pad) + vy - minY) / (maxY - minY);
    return mix(
      materialA,
      materialB,
      Math.round(clamp((x * 0.4 + y) / 1.16) * 63) / 63,
    );
  };
  if (surface === "flat") {
    if (c.treatment === "gradient-face") {
      const rgba = new Uint8Array(64 * 64 * 4);
      for (let j = 0; j < 64; j++)
        for (let i = 0; i < 64; i++)
          rgba.set(
            [...rgb(gradientAt((i + 0.5) / 64, (j + 0.5) / 64)), 255],
            (j * 64 + i) * 4,
          );
      foregroundPlane = { width: 64, height: 64, rgba };
    } else if (c.treatment === "neon-line") {
      // Reference outline strokes the full even-odd path, including aperture boundaries.
      groups.set("outline", { d: "", fill: "none" });
    } else add(rect(0, 0, 512, 512), faceColor, "face");
  } else if (dither) {
    const a = textureHigh,
      b = textureLow;
    const variants = [
      "ordered",
      "chunky",
      "fine",
      "diffusion",
      "halftone",
      "phosphor",
      "handheld",
      "grain",
      "duotone",
    ];
    const vi = variants.indexOf(surface.slice(7)),
      N = FACE_GEOMETRY.effects.dither_grids[vi] ?? 48;
    const val = new Float32Array(N * N),
      on = new Uint8Array(N * N);
    if (vi === 3) {
      for (let j = 0; j < N; j++)
        for (let i = 0; i < N; i++) {
          const u = (i + 0.5) / N,
            v = (j + 0.5) / N;
          val[j * N + i] = maskSampler.sample(u, v)
            ? 0.2 + 0.8 * lightAt(u, v)
            : 0.2 * (1 - v);
        }
      for (let j = 0; j < N; j++)
        for (let i = 0; i < N; i++) {
          const k = j * N + i,
            o = val[k]! > 0.5 ? 1 : 0,
            e = val[k]! - o;
          on[k] = o;
          if (i + 1 < N) val[k + 1]! += (e * 7) / 16;
          if (j + 1 < N) {
            if (i > 0) val[k + N - 1]! += (e * 3) / 16;
            val[k + N]! += (e * 5) / 16;
            if (i + 1 < N) val[k + N + 1]! += e / 16;
          }
        }
    }
    for (let j = 0; j < N; j++)
      for (let i = 0; i < N; i++) {
        const u = (i + 0.5) / N,
          v = (j + 0.5) / N,
          inside = maskSampler.sample(u, v),
          L =
            effect === "moving-light"
              ? 0.5 -
                0.75 *
                  ((u - 0.5) * Math.cos(t * 1.1) +
                    (v - 0.5) * Math.sin(t * 1.1))
              : lightAt(u, v);
        const x = Math.floor((i * 512) / N),
          y = Math.floor((j * 512) / N),
          w = Math.floor(((i + 1) * 512) / N) - x,
          ht = Math.floor(((j + 1) * 512) / N) - y;
        if (
          effect === "pixel-assembly" &&
          noise(j * N + i, 99) > Math.min(1, t / 1.7)
        )
          continue;
        if (inside) {
          let color = mix(b, a, 0.25 + 0.75 * L > bayer(i, j) ? 1 : 0);
          if (vi === 2)
            color = mix(b, a, 0.15 + 0.85 * L > bayer(i, j) ? 1 : 0);
          if (vi === 3) color = on[j * N + i] ? a : b;
          if (vi === 6) {
            const pal = [
              mix(b, "#000000", 0.75),
              mix(b, a, 0.25),
              mix(b, a, 0.65),
              mix(a, "#ffffff", 0.15),
            ];
            color =
              pal[
                Math.min(3, Math.floor((0.45 + 0.55 * L) * 3 + bayer(i, j)))
              ]!;
          }
          if (vi === 7)
            color =
              0.3 + 0.7 * L > noise(i, j, 7)
                ? mix(a, "#ffffff", 0.35)
                : mix(b, "#000000", 0.3);
          if (vi === 8) color = ink;
          if (vi === 5)
            add(
              dot(
                ((i + 0.5) * 512) / N,
                ((j + 0.5) * 512) / N,
                (512 / N) * 0.9,
              ),
              a,
              "face",
              0.12,
            );
          if (vi === 4) {
            add(
              dot(
                ((i + 0.5) * 512) / N,
                ((j + 0.5) * 512) / N,
                (512 / N) * 0.56 * Math.sqrt(0.25 + 0.75 * L),
              ),
              mix(a, b, v),
              "face",
            );
            continue;
          }
          if (vi === 5)
            color =
              0.2 + 0.8 * L > bayer(i, j)
                ? mix(a, "#ffffff", 0.25)
                : mix(b, "#000000", 0.35);
          add(rect(x, y, w, ht), color, "face");
        } else if (!maskSampler.sample(u, v)) {
          const show =
            vi === 1
              ? 0.55 * (1 - v) > bayer(i, j)
              : vi === 2
                ? 0.45 * (1 - Math.hypot(u - 0.5, v - 1.05)) > bayer(i, j)
                : vi === 3
                  ? !!on[j * N + i]
                  : vi === 6
                    ? true
                    : vi === 7
                      ? 0.14 * (1 - v) > noise(i, j, 7)
                      : vi === 8
                        ? (u + v) / 2 > bayer(i, j)
                        : false;
          if (vi === 4) {
            const radius = (512 / N) * 0.5 * 0.22 * (1 - v);
            if (radius >= 0.6)
              add(
                dot(((i + 0.5) * 512) / N, ((j + 0.5) * 512) / N, radius),
                mix(b, "#000000", 0.5),
                "outside",
              );
          }
          if (show)
            add(rect(x, y, w, ht), mix(b, a, vi === 8 ? 0.2 : 0.12), "outside");
        }
      }
    if (vi === 5) {
      for (let y = 0; y < 512; y += 4)
        add(rect(0, y, 512, 2), "#000000", "face", 0.38);
      for (let j = 0; j < 32; j++)
        for (let i = 0; i < 32; i++) {
          const opacity =
            clamp(
              (Math.hypot((i + 0.5) / 32 - 0.5, (j + 0.5) / 32 - 0.5) - 0.3) /
                0.45,
            ) * 0.65;
          if (opacity)
            add(rect(i * 16, j * 16, 16, 16), "#000000", "face", n(opacity));
        }
    }
  } else if (ascii) {
    const a = textureHigh,
      b = textureLow;
    const variants = [
      "shaded",
      "blocks",
      "braille",
      "binary",
      "name",
      "outline",
      "gradient",
      "hatching",
      "inverted",
    ];
    const vi = variants.indexOf(surface.slice(6)),
      C =
        effect === "rain"
          ? 26
          : effect === "character-churn" ||
              effect === "scan" ||
              effect === "typing"
            ? 30
            : (FACE_GEOMETRY.effects.ascii_columns[vi] ?? 30),
      R = Math.floor(C * 0.6),
      cw = 512 / C,
      ch = cw / 0.6,
      oy = (512 - R * ch) / 2;
    const ramp = (L: number, r = FACE_GEOMETRY.effects.ascii_ramp) =>
      r[Math.min(r.length - 1, 1 + Math.floor(L * (r.length - 1)))]!;
    let nameIndex = 0;
    for (let j = 0; j < R; j++)
      for (let i = 0; i < C; i++) {
        const u = (i + 0.5) / C,
          v = (j + 0.5) / R,
          inside = maskSampler.sample(u, v),
          L = lightAt(u, v),
          x = i * cw,
          y = oy + j * ch;
        let char = inside ? ramp(0.2 + 0.8 * L) : "",
          color = a,
          clip: "face" | "outside" = inside ? "face" : "outside",
          opacity = 1;
        if (vi === 1) char = inside ? ramp(0.15 + 0.85 * L, " ░▒▓█") : "";
        if (vi === 2) {
          for (let dy = 0; dy < 4; dy++)
            for (let dx = 0; dx < 2; dx++)
              if (
                maskSampler.sample(
                  (i + (dx + 0.5) / 2) / C,
                  (j + (dy + 0.5) / 4) / R,
                )
              )
                add(
                  dot(
                    x + ((dx + 0.5) * cw) / 2,
                    y + ((dy + 0.5) * ch) / 4,
                    Math.min(cw / 8, ch / 12),
                  ),
                  mix(a, b, v),
                  "face",
                );
          continue;
        }
        if (vi === 3) {
          char = inside ? (noise(i, j) > 0.5 ? "1" : "0") : "";
          opacity = 0.45 + 0.55 * L;
        }
        if (vi === 4) char = inside ? "nyxid·"[nameIndex++ % 6]! : "";
        if (vi === 5) {
          char =
            inside &&
            [
              [1, 0],
              [-1, 0],
              [0, 1],
              [0, -1],
            ].some(
              ([dx = 0, dy = 0]) => !maskSampler.sample(u + dx / C, v + dy / R),
            )
              ? "#"
              : !maskSampler.sample(u, v) && (i + j) % 4 === 0
                ? "·"
                : "";
          color = inside ? a : mix(b, "#000000", 0.4);
        }
        if (vi === 6) color = mix(a, b, (u + v) / 2);
        if (vi === 7) {
          char = inside ? (L > 0.72 ? "/" : L > 0.5 ? "X" : "#") : "";
          color = mix(a, b, L > 0.72 ? 0 : L > 0.5 ? 0.4 : 0.7);
        }
        if (vi === 8) {
          char = maskSampler.sample(u, v)
            ? ""
            : ramp(0.15 + 0.7 * (1 - v), " .:-=+*");
          color = a;
          clip = "outside";
        }
        if (effect === "character-churn" && inside) {
          const chars = FACE_GEOMETRY.effects.churn_chars;
          char =
            chars[Math.floor(noise(i, j, Math.floor(t * 9)) * chars.length)]!;
          opacity = 0.5 + 0.5 * L;
        }
        if (effect === "rain") {
          const head =
              (t * (6 + noise(i, 1) * 10) + noise(i, 2) * 40) % (R + 14),
            d = head - j,
            trail = d >= 0 && d < 9 ? 1 - d / 9 : 0;
          if (!inside && !trail) char = "";
          else {
            char =
              d >= 0 && d < 1
                ? "x"
                : "01nyxid"[Math.floor(noise(i, j, Math.floor(t * 12)) * 7)]!;
            opacity = inside ? Math.max(0.42, trail) : trail * 0.45;
            color = d >= 0 && d < 1 ? "#ecfdf5" : a;
          }
        }
        if (effect === "scan") {
          const line = Math.min(1.15, t / 2.1) * R;
          char =
            j + 0.5 < line ? char : Math.abs(j + 0.5 - line) < 0.7 ? "#" : "";
        }
        if (effect === "typing" && j * C + i > Math.floor(t * 140) % 760)
          char = "";
        if (char && char !== " ")
          add(
            glyph(char, x + cw * 0.12, y + ch * 0.06, cw * 0.76, ch * 0.88),
            color,
            clip,
            n(opacity),
          );
      }
  }
  const paths: DrawPath[] = [...groups.values()].filter((p) => p.d.length > 0);
  if (identityTint && !lightIdentity)
    paths.push({
      d: dot(256, 256, 255),
      fill: "none",
      stroke: coreTint!.fg,
      width: 2,
      opacity: 0.3,
    });
  if (animated && effect === "channel-offset" && t < 0.8) {
    for (const [shift, color] of [
      [-2, a],
      [2, b],
    ] as const)
      paths.push({
        d: polygonPaths([...h.contours, ...eye.contours], (shift * vs) / 512),
        fill: color,
        opacity: 0.18 * (1 - t / 0.8),
        source: true,
      });
  }
  if (c.treatment === "neon-line" && surface === "flat") {
    for (const [spread, opacity] of [
      [3, 0.04],
      [2, 0.08],
      [1.4, 0.14],
    ] as const)
      paths.push({
        d: mask,
        fill: "none",
        stroke: b,
        width: (vs / 24) * spread,
        opacity,
        source: true,
      });
    paths.push({
      d: mask,
      fill: "none",
      stroke: a,
      width: vs / 24,
      source: true,
    });
  }
  if (!options.resting && state !== "idle" && reaction.id !== "still") {
    const color = state === "failure" ? "#a1a1aa" : a;
    paths.push({
      d: dot(256, 256, 238),
      fill: "none",
      ...(activityMotion ? { role: "activity-ring" as const } : {}),
      stroke: color,
      width: 4,
      opacity: reaction.still ? 0.65 : ringOpacity,
    });
  }
  const motion =
    activityMotion?.transform ??
    faceMotion(effect, t, options.level, state, animated);
  const result: DrawList = {
    version: 1,
    mask,
    headMask: h.d,
    outsideMask:
      rect(0, 0, 512, 512) +
      polygonPaths(
        [...h.contours, ...eye.contours].map((p) =>
          p.map(
            ([x, y]) =>
              [(x - vx) * scale + offset, (y - vy) * scale + offset] as Point,
          ),
        ),
      ),
    transform,
    frame: (c.frame ?? "circle") as DrawList["frame"],
    paths,
    backgroundPaths,
    backgroundPlane,
    foregroundPlane,
    state,
    surface,
    fallback: resolvedSurface.fallback,
    contrastAdjusted,
    reaction: effect,
    motion,
  };
  if (!animated) {
    const cost = drawBytes(result);
    while (
      stills.size &&
      (stills.size >= STILL_LIMIT || stillBytes + cost > STILL_BYTES)
    ) {
      const oldest = stills.keys().next().value!;
      stillBytes -= drawBytes(stills.get(oldest)!);
      stills.delete(oldest);
    }
    if (cost <= STILL_BYTES) {
      stills.set(key, result);
      stillBytes += cost;
    }
  }
  return result;
}
export function stillCacheInfo() {
  return {
    entries: stills.size,
    maxEntries: STILL_LIMIT,
    bytes: stillBytes,
    maxBytes: STILL_BYTES,
  };
}
