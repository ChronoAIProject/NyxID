import { useEffect, useMemo, useRef, useState } from "react";
import { FACE_GEOMETRY } from "../../../../../shared/agent-avatars/draw";
import {
  cachedVoiceRippleFrames,
  prepareVoiceRippleFrames,
  voiceRippleDots,
  voiceFace,
  VOICE_RIPPLE_FRAMES,
  VOICE_RIPPLE_PERIOD,
  claimVoiceRipple,
  releaseVoiceRipple,
  subscribeVoiceRipple,
  type VoiceRippleFrames,
} from "../../../../../shared/agent-avatars/voice-ripple";
import type { AvatarConfig } from "@/lib/assistant/agent-avatar-model";
import {
  useAnimationActive,
  useReducedAvatarMotion,
} from "@/lib/assistant/use-avatar-environment";
import { AvatarRenderer } from "./avatar-renderer";
import { animateAvatar } from "./scheduler";
interface Props {
  config: AvatarConfig;
  kind?: string;
  name?: string;
  connection?: "connecting" | "reconnecting" | "connected";
  status: string;
  inputAccepting: boolean;
  outputPlaying: boolean;
  outputLevel?: number;
}
/** Decorative field; existing voice status text remains the live announcement. */
export function VoiceRippleBanner({
  config,
  kind,
  name = "NyxBot",
  connection,
  status,
  inputAccepting,
  outputPlaying,
  outputLevel = 0,
}: Props) {
  const ref = useRef<HTMLDivElement>(null),
    canvas = useRef<HTMLCanvasElement>(null),
    level = useRef(outputLevel),
    playing = useRef(outputPlaying),
    owner = useRef(Symbol("voice-ripple"));
  const reduced = useReducedAvatarMotion() || config.choices.motion === "still",
    visible = useAnimationActive(ref),
    palette = config.choices.ripple_palette ?? "nyx",
    p = FACE_GEOMETRY.ripples[palette] ?? FACE_GEOMETRY.ripples.nyx!,
    eligible = visible && !reduced && (inputAccepting || outputPlaying);
  const [leased, setLeased] = useState(false),
    [prepared, setPrepared] = useState<VoiceRippleFrames | undefined>(() =>
      cachedVoiceRippleFrames(palette),
    ),
    [painted, setPainted] = useState<string>();
  const frames = prepared?.palette === palette ? prepared : undefined,
    dots = useMemo(() => voiceRippleDots(palette), [palette]);
  const faceConfig = useMemo(() => voiceFace(config), [config]);
  useEffect(() => {
    level.current = outputLevel;
    playing.current = outputPlaying;
  }, [outputLevel, outputPlaying]);
  useEffect(() => {
    if (!eligible) {
      setLeased(false);
      return;
    }
    const ownerId = owner.current;
    const refresh = () => setLeased(claimVoiceRipple(ownerId)),
      off = subscribeVoiceRipple(refresh);
    refresh();
    return () => {
      off();
      releaseVoiceRipple(ownerId);
    };
  }, [eligible]);
  useEffect(() => {
    let live = true;
    const present = cachedVoiceRippleFrames(palette);
    setPrepared(present);
    if (eligible && leased && !present)
      prepareVoiceRippleFrames(palette)
        .then((result) => {
          if (live) setPrepared(result);
        })
        .catch(() => {
          if (live) ref.current?.setAttribute("data-ripple-problem", "prepare");
        });
    return () => {
      live = false;
    };
  }, [palette, eligible, leased]);
  useEffect(() => {
    if (!frames) return;
    let live = true,
      stop: undefined | (() => void);
    const images = frames.frames.map((uri) => {
      const img = new Image();
      img.src = uri;
      return img;
    });
    let smoothedLevel = 0;
    let previousPaint = performance.now();
    const paint = (index: number) => {
      const safe =
          ((Math.floor(index) % images.length) + images.length) % images.length,
        image = images[safe],
        context = canvas.current?.getContext("2d");
      if (!context || !image?.complete || !image.naturalWidth) return;
      const now = performance.now();
      const elapsed = Math.max(0, Math.min(0.25, (now - previousPaint) / 1000));
      previousPaint = now;
      const target = Number.isFinite(level.current)
        ? Math.max(0, Math.min(1, level.current))
        : 0;
      const easing = target > smoothedLevel ? 0.08 : 0.24;
      smoothedLevel +=
        (target - smoothedLevel) * (1 - Math.exp(-elapsed / easing));
      context.clearRect(0, 0, 384, 128);
      context.globalAlpha =
        eligible && leased && playing.current ? 0.55 + 0.45 * smoothedLevel : 1;
      context.drawImage(image, 0, 0, 384, 128);
      canvas.current?.setAttribute("data-ripple-frame", `${palette}:${safe}`);
    };
    Promise.all(images.map((image) => image.decode()))
      .then(() => {
        if (!live) return;
        paint(3);
        setPainted(palette);
        if (eligible && leased)
          stop = animateAvatar(
            true,
            (t) =>
              paint(
                Math.floor(
                  (Math.max(0, t) / VOICE_RIPPLE_PERIOD) * VOICE_RIPPLE_FRAMES,
                ),
              ),
            10,
            () => paint(3),
          );
      })
      .catch(() => {
        if (live) {
          setPainted(undefined);
          ref.current?.setAttribute("data-ripple-problem", "decode");
        }
      });
    return () => {
      live = false;
      stop?.();
    };
  }, [frames, palette, eligible, leased]);
  return (
    <div
      ref={ref}
      aria-hidden="true"
      data-voice-ripple={palette}
      data-voice-ripple-moving={eligible && leased}
      className="relative mx-auto flex aspect-[3/1] w-full max-w-[600px] items-center overflow-hidden rounded-3xl"
      style={{ backgroundColor: p.bg, color: p.text }}
    >
      <svg
        viewBox="0 0 384 128"
        preserveAspectRatio="none"
        className="absolute inset-0 size-full"
      >
        <rect width="384" height="128" fill={p.bg} />
        {dots.map((dot, i) => (
          <circle
            key={i}
            cx={dot.x}
            cy={dot.y}
            r={dot.r}
            fill={dot.color}
            opacity={dot.opacity}
          />
        ))}
      </svg>
      <canvas
        key={palette}
        ref={canvas}
        width={384}
        height={128}
        className="absolute inset-0 size-full"
        style={{ opacity: painted === palette ? 1 : 0 }}
      />
      <div
        className="relative ml-[5%] flex aspect-square w-[24%] shrink-0 items-center justify-center rounded-full"
        style={{
          backgroundColor: p.ring,
          boxShadow: connection ? `0 0 0 3px ${p.a}` : undefined,
        }}
      >
        <AvatarRenderer
          config={faceConfig}
          kind={kind}
          pixels={112}
          theme={p.light ? "light" : "dark"}
          ambient={p.ring}
          mood={
            outputPlaying ? "speaking" : inputAccepting ? "listening" : "idle"
          }
          animated={inputAccepting || outputPlaying}
          className="block size-full"
        />
      </div>
      <div className="relative ml-[5%] min-w-0 pr-4">
        <p className="text-balance type-card-title">{status}</p>
        <p className="mt-1 type-secondary">{name}</p>
      </div>
    </div>
  );
}
