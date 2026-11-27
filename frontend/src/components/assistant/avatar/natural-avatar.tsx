import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { drawAvatar } from "../../../../../shared/agent-avatars/draw";
import {
  emoteIndex,
  eyePoseMasks,
  RESTING_EYES,
  supportsNaturalEyes,
  type EmotePose,
} from "../../../../../shared/agent-avatars/emotes";
import {
  propTransform,
  staticPropPose,
} from "../../../../../shared/agent-avatars/props";
import { hashId, processState } from "@/lib/assistant/agent-avatar-model";
import {
  useAnimationActive,
  useReducedAvatarMotion,
} from "@/lib/assistant/use-avatar-environment";
import { AvatarSvg } from "./avatar-svg";
import { animateRiveEyes } from "./rive-motion";
import type { AvatarRendererProps } from "./avatar-renderer";
import { useCreatedAvatarMood } from "./use-created-avatar-mood";
import { ActivityProps } from "./activity-props";
import { paintPixelScene, pixelGrid } from "./pixel-scene";

/** Rive authors the performance; the shared painter preserves every chosen material. */
export function NaturalAvatar({
  config,
  agentId,
  kind,
  mood: observedMood = "idle",
  theme,
  ambient,
  pixels = 112,
  animated = true,
  className,
  label,
  eventKey,
  connection,
}: AvatarRendererProps) {
  const root = useRef<HTMLSpanElement>(null);
  const canvas = useRef<HTMLCanvasElement>(null);
  const [pixelReady, setPixelReady] = useState(false);
  const pixelStatus = useRef(false);
  const reduced = useReducedAvatarMotion();
  const visible = useAnimationActive(root);
  const mood = useCreatedAvatarMood(
    agentId,
    observedMood,
    animated && visible && pixels >= 24,
  );
  const [ready, setReady] = useState(false);
  const [retryVersion, setRetryVersion] = useState(0);
  const retries = useRef(0);
  const retryTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  const session = useRef<ReturnType<typeof animateRiveEyes>>(null);
  const eyeMotion = supportsNaturalEyes(config.choices.surface ?? "flat");
  const propsVisible =
    pixels >= 44 &&
    config.choices.motion !== "still" &&
    config.choices[`react_${processState(mood)}`] !== "still";
  const enabled =
    animated &&
    pixels >= 24 &&
    !reduced &&
    (eyeMotion || propsVisible) &&
    config.choices.motion !== "still" &&
    config.choices[`react_${processState(mood)}`] !== "still";
  const current = useRef({ config, mood });
  useLayoutEffect(() => {
    current.current = { config, mood };
  }, [config, mood]);
  const scene = useMemo(
    () =>
      drawAvatar(config, {
        kind,
        theme,
        ambient,
        size: pixels,
        resting: true,
        reduced: true,
      }),
    [config, kind, theme, pixels, ambient],
  );
  const material = useMemo(
    () =>
      !eyeMotion || scene.surface === "flat"
        ? undefined
        : drawAvatar(config, {
            kind,
            theme,
            ambient,
            size: pixels,
            resting: true,
            reduced: true,
            materialEyes: true,
          }),
    [config, kind, theme, pixels, scene.surface, eyeMotion, ambient],
  );
  const lastPose = useRef<EmotePose>(RESTING_EYES);
  const paint = useRef<(pose: EmotePose) => void>(() => {});
  useLayoutEffect(() => {
    paint.current = (pose) => {
      if (!root.current) return;
      const masks = eyePoseMasks(
        scene,
        config.choices.head ?? "round",
        config.choices.eyes ?? "tall",
        eyeMotion ? pose : RESTING_EYES,
      );
      root.current
        .querySelectorAll("[data-avatar-eye-mask], [data-avatar-source-mask]")
        .forEach((path) => path.setAttribute("d", masks.mask));
      root.current
        .querySelectorAll("[data-avatar-outside-mask]")
        .forEach((path) => path.setAttribute("d", masks.outsideMask));
      const props =
        config.choices.motion === "quiet"
          ? staticPropPose(mood)
          : (pose.props ?? staticPropPose(mood));
      root.current.querySelectorAll("[data-rive-prop]").forEach((element) => {
        const value = props[element.getAttribute("data-rive-prop")!];
        const transform = value ? propTransform(value) : "scale(0)";
        if (element.getAttribute("transform") !== transform)
          element.setAttribute("transform", transform);
      });
      lastPose.current = pose;
      try {
        const context = canvas.current?.getContext("2d", {
          willReadFrequently: true,
        });
        if (!context) return;
        paintPixelScene(context, {
          scene,
          materialScene: material,
          config,
          mood,
          pose,
          theme,
          pixels,
          eyes: eyeMotion,
          props: propsVisible,
          quiet: config.choices.motion === "quiet",
        });
        if (!pixelStatus.current) {
          pixelStatus.current = true;
          setPixelReady(true);
        }
      } catch {
        if (pixelStatus.current) {
          pixelStatus.current = false;
          setPixelReady(false);
        }
      }
    };
  }, [config, scene, material, eyeMotion, mood, pixels, propsVisible, theme]);
  useEffect(() => {
    if (!enabled) return;
    if (session.current) {
      session.current.active(visible);
      return;
    }
    if (!visible) return;
    let failedSynchronously = false;
    let active: ReturnType<typeof animateRiveEyes> = null;
    active = animateRiveEyes(
      {
        ready: (value) => {
          setReady(value);
          if (!value) {
            paint.current(RESTING_EYES);
            if (active) {
              active.stop();
              if (session.current === active) session.current = null;
            } else failedSynchronously = true;
            if (retries.current < 2 && retryTimer.current === undefined) {
              retries.current += 1;
              retryTimer.current = setTimeout(() => {
                retryTimer.current = undefined;
                setRetryVersion((version) => version + 1);
              }, retries.current * 2000);
            }
          }
        },
        pose: (pose) => paint.current(pose),
      },
      emoteIndex(current.current.mood),
      emoteIndex(current.current.mood) === 0
        ? (hashId(agentId ?? "preview") % 1500) / 1000
        : 0,
    );
    if (failedSynchronously) active?.stop();
    else session.current = active;
  }, [enabled, visible, agentId, eventKey, retryVersion]);
  useEffect(
    () => () => {
      session.current?.stop();
      session.current = null;
      clearTimeout(retryTimer.current);
      retryTimer.current = undefined;
      retries.current = 0;
      setReady(false);
    },
    [enabled, agentId, eventKey],
  );
  useEffect(() => {
    session.current?.state(emoteIndex(mood));
  }, [mood, ready]);
  useLayoutEffect(() => {
    paint.current(enabled && ready ? lastPose.current : RESTING_EYES);
  }, [scene, enabled, ready, mood, propsVisible, pixels]);
  return (
    <span
      ref={root}
      className={className}
      style={{ position: "relative" }}
      data-avatar-renderer={ready ? "rive" : "vector"}
      data-avatar-motion={enabled && visible ? "natural" : "static"}
      data-avatar-state={processState(mood)}
      data-avatar-reaction={enabled ? "natural" : "still"}
      data-avatar-connection={connection}
      data-avatar-motion-fallback={!eyeMotion ? "texture" : undefined}
      data-avatar-style="pixel"
    >
      <span
        className="block size-full"
        aria-hidden={pixelReady ? true : undefined}
        style={{ opacity: pixelReady ? 0 : 1 }}
      >
        <AvatarSvg
          scene={scene}
          eyeMaterial={enabled ? material : undefined}
          label={label}
          className="block size-full"
        />
        {propsVisible ? (
          <ActivityProps
            mood={mood}
            compact={pixels < 96}
            config={config}
            theme={theme}
            surface={scene.surface}
          />
        ) : null}
      </span>
      <canvas
        ref={canvas}
        width={pixelGrid(pixels)}
        height={pixelGrid(pixels)}
        data-avatar-pixel=""
        data-pixel-grid={pixelGrid(pixels)}
        role={label ? "img" : undefined}
        aria-label={label}
        aria-hidden={!label || !pixelReady}
        className="pointer-events-none absolute inset-0 block size-full"
        style={{ imageRendering: "pixelated", opacity: pixelReady ? 1 : 0 }}
      />
    </span>
  );
}
