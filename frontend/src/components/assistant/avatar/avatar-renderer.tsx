import {
  createdAvatarEvent,
  claimCreatedAvatar,
  subscribeCreatedAvatars,
} from "../../../../../shared/agent-avatars/created";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { drawAvatar } from "../../../../../shared/agent-avatars/draw";
import {
  reactionFor,
  AVATAR_CATALOGUE,
  processState,
  type AvatarConfig,
  type AvatarMood,
  type AvatarTheme,
} from "@/lib/assistant/agent-avatar-model";
import {
  useAnimationActive,
  useReducedAvatarMotion,
} from "@/lib/assistant/use-avatar-environment";
import { AvatarSvg } from "./avatar-svg";
import { paintAvatar } from "./canvas";
import { animateAvatar } from "./scheduler";
import { NaturalAvatar } from "./natural-avatar";
export interface AvatarRendererProps {
  config: AvatarConfig;
  agentId?: string;
  kind?: string;
  mood?: AvatarMood;
  theme: AvatarTheme;
  ambient?: string;
  className?: string;
  label?: string;
  pixels?: number;
  level?: number;
  animated?: boolean;
  connection?: "connecting" | "reconnecting" | "connected";
  eventKey?: string;
  performance?: "natural" | "expressive";
}
export function AvatarRenderer(props: AvatarRendererProps) {
  const state = processState(props.mood);
  const selected = props.config.choices[`react_${state}`];
  const custom =
    selected &&
    selected !== "still" &&
    selected !== AVATAR_CATALOGUE.default_reactions[state];
  return props.performance === "expressive" ||
    (props.performance !== "natural" && custom) ? (
    <ExpressiveAvatarRenderer {...props} />
  ) : (
    <NaturalAvatar {...props} />
  );
}
function ExpressiveAvatarRenderer({
  config,
  kind,
  mood: observedMood = "idle",
  agentId,
  theme,
  ambient,
  className,
  label,
  pixels = 112,
  level = 0,
  animated = true,
  eventKey,
  connection,
}: AvatarRendererProps) {
  const ref = useRef<HTMLSpanElement>(null),
    canvas = useRef<HTMLCanvasElement>(null);
  const reduced = useReducedAvatarMotion(),
    visible = useAnimationActive(ref);
  const candidate = useSyncExternalStore(
    subscribeCreatedAvatars,
    () => createdAvatarEvent(agentId),
    () => null,
  );
  const claimant = useRef(Symbol("focused-avatar"));
  const [created, setCreated] = useState<string | null>(null);
  const yielded = useRef<string | null>(null);
  useEffect(() => {
    if (!candidate) {
      setCreated(null);
      return;
    }
    const eligible = visible && animated && pixels >= 24;
    if (yielded.current === candidate.id) {
      setCreated(null);
      return;
    }
    if (observedMood !== "idle") {
      if (
        (eligible || created === candidate.id) &&
        claimCreatedAvatar(candidate, claimant.current)
      )
        yielded.current = candidate.id;
      setCreated(null);
      return;
    }
    if (!eligible) {
      setCreated((previous) => (previous === candidate.id ? previous : null));
      return;
    }
    if (claimCreatedAvatar(candidate, claimant.current))
      setCreated(candidate.id);
    else setCreated(null);
  }, [candidate, visible, animated, pixels, observedMood, created]);
  const creation =
    observedMood === "idle" && created === candidate?.id ? created : null;
  const mood = creation ? "created" : observedMood;
  const processEventKey = `${mood}:${creation ?? eventKey ?? "state"}`;
  const [failed, setFailed] = useState(false),
    [ready, setReady] = useState(false);
  const [seconds, setSeconds] = useState(0),
    levelRef = useRef(level);
  const progress = useRef({ key: processEventKey, seconds: 0, done: false });
  useEffect(() => {
    levelRef.current = level;
  }, [level]);
  const reaction = reactionFor(config, mood, pixels, false, reduced);
  const rich =
    pixels >= 96 &&
    config.choices.motion !== "quiet" &&
    (config.choices.surface !== "flat" ||
      config.choices.background === "ripple" ||
      [
        "halftone-ripple",
        "character-churn",
        "rain",
        "typing",
        "scan",
        "face-shuffle",
        "channel-offset",
      ].includes(reaction.effective));
  const scene = useMemo(
    () =>
      drawAvatar(config, {
        size: pixels,
        kind,
        mood,
        theme,
        ambient,
        reduced,
        seconds: rich ? 0 : seconds,
        level,
      }),
    [config, kind, pixels, mood, theme, ambient, reduced, rich, seconds, level],
  );
  useEffect(() => {
    const key = processEventKey;
    if (progress.current.key !== key)
      progress.current = { key, seconds: 0, done: false };
    if (
      !visible ||
      reduced ||
      !animated ||
      reaction.still ||
      reaction.activityMotion?.period === null ||
      pixels < 24 ||
      progress.current.done ||
      failed
    )
      return;
    const elapsed = progress.current.seconds,
      duration = reaction.duration;
    let unregister = () => {};
    unregister = animateAvatar(
      rich,
      (t) => {
        const sample = reaction.oneShot
          ? Math.min(elapsed + t, duration)
          : elapsed + t;
        progress.current.seconds = sample;
        if (rich) {
          const context = canvas.current?.getContext("2d");
          if (context)
            paintAvatar(
              context,
              drawAvatar(config, {
                size: pixels,
                kind,
                mood,
                theme,
                ambient,
                seconds: sample,
                level: levelRef.current,
              }),
              pixels * 2,
            );
        } else setSeconds(sample);
        if (reaction.oneShot && sample >= duration) {
          progress.current.done = true;
          unregister();
        }
      },
      0,
      () => {
        setFailed(true);
        setReady(false);
      },
    );
    return unregister;
  }, [
    failed,
    visible,
    reduced,
    animated,
    reaction.still,
    reaction.activityMotion,
    reaction.oneShot,
    reaction.state,
    reaction.duration,
    pixels,
    rich,
    config,
    kind,
    mood,
    theme,
    ambient,
    processEventKey,
  ]);
  useEffect(() => {
    if (!rich) return;
    try {
      const context = canvas.current?.getContext("2d");
      if (!context) throw new Error("canvas unavailable");
      paintAvatar(
        context,
        drawAvatar(config, {
          size: pixels,
          kind,
          mood,
          theme,
          ambient,
          reduced,
          seconds: progress.current.seconds,
          level: levelRef.current,
        }),
        pixels * 2,
      );
      setFailed(false);
      setReady(true);
    } catch {
      setFailed(true);
      setReady(false);
    }
  }, [rich, scene, pixels, config, kind, mood, theme, ambient, reduced]);

  return (
    <span
      ref={ref}
      style={{ position: "relative" }}
      className={className}
      data-avatar-connection={connection}
      data-avatar-renderer={failed ? "static" : rich ? "canvas" : "vector"}
      data-avatar-state={scene.state}
      data-avatar-reaction={scene.reaction}
    >
      {connection ? (
        <span
          aria-hidden="true"
          className={`pointer-events-none absolute inset-0 rounded-full border-2 border-nyx-400/60 ${connection === "connected" ? "" : "border-dashed"}`}
        />
      ) : null}
      {rich ? (
        <>
          {!ready || failed ? (
            <AvatarSvg
              scene={scene}
              label={label}
              className="block size-full"
            />
          ) : null}
          <canvas
            ref={canvas}
            width={pixels * 2}
            height={pixels * 2}
            role={label ? "img" : undefined}
            aria-label={label}
            aria-hidden={!label || !ready || failed}
            style={{
              position: "absolute",
              inset: 0,
              opacity: ready && !failed ? 1 : 0,
            }}
            className="block size-full"
          />
        </>
      ) : (
        <AvatarSvg scene={scene} label={label} className="block size-full" />
      )}
    </span>
  );
}
