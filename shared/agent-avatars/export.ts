import { encodePng, encodePngAsync } from "./png";
/** Deterministic portable PNG encoder, shared by browsers and native. Uses
 * uncompressed DEFLATE: no system fonts, codecs, timestamps, RNG or metadata. */
import { contours } from "./svg-path.js";
import { drawAvatar, type DrawList } from "./draw";
import type { AvatarConfig, AvatarTheme } from "./model";
type Point = readonly [number, number];
const SIZE = 512;
function* coverageSteps(
  paths: readonly (readonly Point[])[],
  nonzero = false,
): Generator<void, Uint8Array> {
  const out = new Uint8Array(SIZE * SIZE);
  let lo = SIZE,
    hi = 0;
  for (const p of paths)
    for (const [, y] of p) {
      lo = Math.min(lo, Math.floor(y));
      hi = Math.max(hi, Math.ceil(y));
    }
  for (let row = Math.max(0, lo); row < Math.min(SIZE, hi); row++) {
    if (row % 8 === 0) yield;
    const y = row + 0.5,
      hits: { x: number; w: number }[] = [];
    for (const p of paths)
      for (let i = 0, j = p.length - 1; i < p.length; j = i++) {
        const a = p[j]!,
          b = p[i]!;
        if (a[1] > y !== b[1] > y)
          hits.push({
            x: a[0] + ((b[0] - a[0]) * (y - a[1])) / (b[1] - a[1]),
            w: b[1] > a[1] ? 1 : -1,
          });
      }
    hits.sort((a, b) => a.x - b.x);
    let winding = 0;
    for (let i = 0; i < hits.length - 1; i++) {
      winding = nonzero ? winding + hits[i]!.w : winding ? 0 : 1;
      if (winding)
        out.fill(
          255,
          row * SIZE + Math.max(0, Math.min(SIZE, Math.ceil(hits[i]!.x - 0.5))),
          row * SIZE +
            Math.max(0, Math.min(SIZE, Math.ceil(hits[i + 1]!.x - 0.5))),
        );
    }
  }
  return out;
}
/** Actual export pixels. Also used by native/web parity and topology tests. */
function finish<T>(steps: Generator<void, T>): T {
  let next = steps.next();
  while (!next.done) next = steps.next();
  return next.value;
}
export function rasterAvatar(scene: DrawList): Uint8Array {
  return finish(rasterAvatarSteps(scene));
}
function* rasterAvatarSteps(scene: DrawList): Generator<void, Uint8Array> {
  const nums = scene.transform.match(/[-+]?\d*\.?\d+/g)!.map(Number),
    scale = nums[2]!,
    tx = nums[0]!,
    ty = nums[1]!;
  const transformed = (d: string) =>
    contours(d).map((p) =>
      p.map(([x, y]) => [x * scale + tx, y * scale + ty] as Point),
    );
  const face = yield* coverageSteps(transformed(scene.mask));
  const outside = yield* coverageSteps(contours(scene.outsideMask));
  const rgba = new Uint8Array(SIZE * SIZE * 4);
  function* paintPlane(
    plane: NonNullable<DrawList["backgroundPlane"]>,
    clipped = false,
  ) {
    for (let y = 0; y < SIZE; y++) {
      if (y % 8 === 0) yield;
      for (let x = 0; x < SIZE; x++) {
        if (clipped && !face[y * SIZE + x]) continue;
        if (
          scene.frame === "circle" &&
          Math.hypot(x + 0.5 - 256, y + 0.5 - 256) > 256
        )
          continue;
        if (
          scene.frame === "tile" &&
          Math.abs(x + 0.5 - 256) > 160 &&
          Math.abs(y + 0.5 - 256) > 160 &&
          Math.hypot(
            Math.abs(x + 0.5 - 256) - 160,
            Math.abs(y + 0.5 - 256) - 160,
          ) > 96
        )
          continue;
        const u = Math.max(
            0,
            Math.min(plane.width - 1, ((x + 0.5) * plane.width) / SIZE - 0.5),
          ),
          v = Math.max(
            0,
            Math.min(plane.height - 1, ((y + 0.5) * plane.height) / SIZE - 0.5),
          );
        const i = Math.floor(u),
          j = Math.floor(v),
          dx = u - i,
          dy = v - j,
          at = (y * SIZE + x) * 4;
        for (let c = 0; c < 4; c++) {
          const a = plane.rgba[(j * plane.width + i) * 4 + c]!,
            b =
              plane.rgba[
                (j * plane.width + Math.min(plane.width - 1, i + 1)) * 4 + c
              ]!,
            d =
              plane.rgba[
                (Math.min(plane.height - 1, j + 1) * plane.width + i) * 4 + c
              ]!,
            e =
              plane.rgba[
                (Math.min(plane.height - 1, j + 1) * plane.width +
                  Math.min(plane.width - 1, i + 1)) *
                  4 +
                  c
              ]!;
          rgba[at + c] = Math.round(
            a * (1 - dx) * (1 - dy) +
              b * dx * (1 - dy) +
              d * (1 - dx) * dy +
              e * dx * dy,
          );
        }
      }
    }
  }

  if (scene.backgroundPlane) yield* paintPlane(scene.backgroundPlane);
  for (const [index, path] of [
    ...scene.backgroundPaths,
    ...scene.paths,
  ].entries()) {
    if (index === scene.backgroundPaths.length && scene.foregroundPlane)
      yield* paintPlane(scene.foregroundPlane, true);
    const shapes = path.source ? transformed(path.d) : contours(path.d);
    let mask: Uint8Array;
    if (path.stroke) {
      const width = ((path.width ?? 1) * (path.source ? scale : 1)) / 2,
        quads: Point[][] = [];
      for (const p of shapes)
        for (let i = 1; i < p.length; i++) {
          const a = p[i - 1]!,
            b = p[i]!,
            len = Math.hypot(b[0] - a[0], b[1] - a[1]);
          if (!len) continue;
          const dx = (-(b[1] - a[1]) / len) * width,
            dy = ((b[0] - a[0]) / len) * width;
          quads.push([
            [a[0] + dx, a[1] + dy],
            [b[0] + dx, b[1] + dy],
            [b[0] - dx, b[1] - dy],
            [a[0] - dx, a[1] - dy],
          ]);
        }
      mask = yield* coverageSteps(quads, true);
    } else mask = yield* coverageSteps(shapes, true);
    const color = path.stroke ?? path.fill;
    if (color === "none") continue;
    const rgb = [1, 3, 5].map((i) => parseInt(color.slice(i, i + 2), 16)),
      alpha = path.opacity ?? 1;
    for (let k = 0; k < mask.length; k++) {
      if (k % 4096 === 0) yield;
      if (
        !mask[k] ||
        (path.clip === "face" && !face[k]) ||
        (path.clip === "outside" && !outside[k])
      )
        continue;
      const x = (k % SIZE) + 0.5,
        y = Math.floor(k / SIZE) + 0.5;
      if (scene.frame === "circle" && Math.hypot(x - 256, y - 256) > 256)
        continue;
      if (
        scene.frame === "tile" &&
        Math.abs(x - 256) > 160 &&
        Math.abs(y - 256) > 160 &&
        Math.hypot(Math.abs(x - 256) - 160, Math.abs(y - 256) - 160) > 96
      )
        continue;
      const offset = k * 4,
        old = rgba[offset + 3]! / 255,
        a = alpha + old * (1 - alpha);
      for (let c = 0; c < 3; c++)
        rgba[offset + c] = Math.round(
          (rgb[c]! * alpha + rgba[offset + c]! * old * (1 - alpha)) / a,
        );
      rgba[offset + 3] = Math.round(a * 255);
    }
  }
  if (!scene.paths.length && scene.foregroundPlane)
    yield* paintPlane(scene.foregroundPlane, true);
  return rgba;
}
export function pngAvatar(
  config: AvatarConfig,
  transparent = false,
  theme: AvatarTheme = "dark",
  kind?: string,
): Uint8Array {
  return encodePng(
    SIZE,
    SIZE,
    rasterAvatar(
      drawAvatar(config, {
        size: SIZE,
        resting: true,
        reduced: true,
        transparent,
        theme,
        kind,
      }),
    ),
  );
}

/** Yield between bounded raster rows and path batches. The synchronous and
 * asynchronous exports drain the exact same composition, with identical bytes. */
export async function pngAvatarAsync(
  config: AvatarConfig,
  transparent = false,
  theme: AvatarTheme = "dark",
  kind?: string,
  options: { check?: () => void; onProgress?: (steps: number) => void } = {},
): Promise<Uint8Array> {
  const pause = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
  await pause();
  options.check?.();
  const scene = drawAvatar(config, {
    size: SIZE,
    theme,
    kind,
    resting: true,
    transparent,
    reduced: true,
  });
  const steps = rasterAvatarSteps(scene);
  let next = steps.next(),
    count = 0,
    started = performance.now();
  while (!next.done) {
    options.check?.();
    count++;
    if (performance.now() - started >= 8) {
      options.onProgress?.(count);
      await pause();
      options.check?.();
      started = performance.now();
    }
    next = steps.next();
  }
  await pause();
  options.check?.();
  return encodePngAsync(SIZE, SIZE, next.value, {
    check: options.check,
    onYield: () => options.onProgress?.(++count),
  });
}
