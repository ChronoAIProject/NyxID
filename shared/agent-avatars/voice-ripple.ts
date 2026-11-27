/** Source radial halftone fields. One cached palette and one cancellable
 * preparation; rendering a banner never synchronously builds a frame bank. */
import { FACE_GEOMETRY, mix } from "./draw";
import { encodePng, pngDataUri } from "./png";
import type { AvatarConfig } from "./model";
export const VOICE_RIPPLE_WIDTH = 384,
  VOICE_RIPPLE_HEIGHT = 128,
  VOICE_RIPPLE_FRAMES = 12,
  VOICE_RIPPLE_PERIOD = (2 * Math.PI) / 4;
const rgb = (hex: string) =>
  [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16));
export interface RippleDot {
  x: number;
  y: number;
  r: number;
  color: string;
  opacity: number;
}
export function voiceRippleDots(
  palette: string,
  seconds = VOICE_RIPPLE_PERIOD / 4,
): RippleDot[] {
  const p = FACE_GEOMETRY.ripples[palette] ?? FACE_GEOMETRY.ripples.nyx!,
    W = VOICE_RIPPLE_WIDTH,
    H = VOICE_RIPPLE_HEIGHT,
    step = H / 16,
    cx = W * 0.17,
    cy = H * 0.5,
    dots: RippleDot[] = [];
  for (let y = step / 2; y < H; y += step)
    for (let x = step / 2; x < W; x += step) {
      const distance = Math.hypot(x - cx, y - cy) / H,
        wave =
          Math.round(
            (0.5 + 0.5 * Math.sin(seconds * 4 - distance * 14)) * 1e6,
          ) / 1e6,
        fade = Math.max(0, 1 - distance * 0.85),
        r = step * 0.46 * (0.18 + 0.82 * wave) * (0.3 + 0.7 * fade);
      if (r >= 0.5)
        dots.push({
          x,
          y,
          r,
          color: mix(p.a, p.b, wave),
          opacity: 0.25 + 0.75 * fade,
        });
    }
  return dots;
}
export function voiceRipplePixels(
  palette: string,
  seconds: number,
): Uint8Array {
  const p = FACE_GEOMETRY.ripples[palette] ?? FACE_GEOMETRY.ripples.nyx!,
    W = VOICE_RIPPLE_WIDTH,
    H = VOICE_RIPPLE_HEIGHT,
    pixels = new Uint8Array(W * H * 4),
    background = rgb(p.bg);
  for (let i = 0; i < W * H; i++) {
    const at = i * 4;
    pixels[at] = background[0]!;
    pixels[at + 1] = background[1]!;
    pixels[at + 2] = background[2]!;
    pixels[at + 3] = 255;
  }
  for (const { x, y, r, color, opacity } of voiceRippleDots(palette, seconds)) {
    const paint = rgb(color);
    for (
      let py = Math.max(0, Math.floor(y - r - 1));
      py < Math.min(H, Math.ceil(y + r + 1));
      py++
    )
      for (
        let px = Math.max(0, Math.floor(x - r - 1));
        px < Math.min(W, Math.ceil(x + r + 1));
        px++
      ) {
        const coverage =
            Math.max(
              0,
              Math.min(1, r + 0.5 - Math.hypot(px + 0.5 - x, py + 0.5 - y)),
            ) * opacity,
          at = (py * W + px) * 4;
        for (let c = 0; c < 3; c++)
          pixels[at + c] = Math.round(
            pixels[at + c]! * (1 - coverage) + paint[c]! * coverage,
          );
      }
  }
  return pixels;
}
export interface VoiceRippleFrames {
  readonly palette: string;
  readonly frames: readonly string[];
  readonly still: string;
  readonly bytes: number;
  readonly buildMs: number;
}
let cached: VoiceRippleFrames | undefined;
interface Preparation {
  palette: string;
  cancelled: boolean;
  promise: Promise<VoiceRippleFrames>;
}
let inflight: Preparation | undefined;
export function cachedVoiceRippleFrames(palette: string) {
  return cached?.palette === palette ? cached : undefined;
}
const yieldFrame = () => new Promise<void>((resolve) => setTimeout(resolve, 0));
export function prepareVoiceRippleFrames(
  palette: string,
): Promise<VoiceRippleFrames> {
  if (cached?.palette === palette) return Promise.resolve(cached);
  if (inflight?.palette === palette && !inflight.cancelled)
    return inflight.promise;
  if (inflight) inflight.cancelled = true;
  const request: Preparation = {
    palette,
    cancelled: false,
    promise: Promise.resolve(undefined as never),
  };
  request.promise = (async () => {
    const frames: string[] = [];
    let buildMs = 0;
    for (let i = 0; i < VOICE_RIPPLE_FRAMES; i++) {
      await yieldFrame();
      if (request.cancelled) throw new Error("ripple preparation cancelled");
      const started = performance.now();
      frames.push(
        pngDataUri(
          encodePng(
            VOICE_RIPPLE_WIDTH,
            VOICE_RIPPLE_HEIGHT,
            voiceRipplePixels(
              palette,
              (i * VOICE_RIPPLE_PERIOD) / VOICE_RIPPLE_FRAMES,
            ),
          ),
        ),
      );
      buildMs += performance.now() - started;
    }
    if (request.cancelled || inflight !== request)
      throw new Error("ripple preparation cancelled");
    cached = {
      palette,
      frames,
      still: frames[3]!,
      bytes: frames.reduce((sum, uri) => sum + uri.length * 2, 0),
      buildMs,
    };
    inflight = undefined;
    return cached;
  })();
  inflight = request;
  return request.promise;
}
export function voiceRippleCacheInfo() {
  return {
    palettes: cached ? 1 : 0,
    bytes: cached?.bytes ?? 0,
    maxPalettes: 1,
    maxBytes: 7 * 1024 * 1024,
    preparing: inflight && !inflight.cancelled ? 1 : 0,
  };
}
export function voiceFace(config: AvatarConfig): AvatarConfig {
  return {
    template: config.template,
    choices: {
      ...config.choices,
      surface: "flat",
      background: "transparent",
      frame: "none",
    },
  };
}
let lease: symbol | undefined;
const listeners = new Set<() => void>();
export function claimVoiceRipple(owner: symbol) {
  if (!lease) {
    lease = owner;
    for (const f of listeners) f();
  }
  return lease === owner;
}
export function releaseVoiceRipple(owner: symbol) {
  if (lease === owner) {
    lease = undefined;
    if (inflight) inflight.cancelled = true;
    for (const f of [...listeners]) f();
  }
}
export function subscribeVoiceRipple(f: () => void) {
  listeners.add(f);
  return () => {
    listeners.delete(f);
  };
}
