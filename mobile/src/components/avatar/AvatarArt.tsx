import Svg, { Circle, Path } from "react-native-svg";
import {
  quietActivityMotion,
  quietEyeFrame,
  quietMotion,
  sampleActivityMotion,
} from "../../../../shared/agent-avatars/quiet-motion";
import { FACE_GEOMETRY } from "../../../../shared/agent-avatars/draw";
import type { SharedValue } from "react-native-reanimated";
import {
  createdAvatarEvent,
  claimCreatedAvatar,
  subscribeCreatedAvatars,
} from "../../../../shared/agent-avatars/created";
import {
  useEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { View, StyleSheet } from "react-native";
import Animated, {
  cancelAnimation,
  useAnimatedStyle,
  useDerivedValue,
  useSharedValue,
  withTiming,
  withRepeat,
  Easing,
} from "react-native-reanimated";
import { drawAvatar } from "../../../../shared/agent-avatars/draw";
import {
  reactionFor,
  type AvatarConfig,
  type AvatarMood,
  type AvatarTheme,
} from "../../lib/avatars/agentAvatars";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useAvatarAnimationActive } from "./useAvatarAnimationActive";
import { AvatarSvg } from "./AvatarSvg";
interface Props {
  kind?: string;
  agentId?: string;
  config: AvatarConfig;
  mood?: AvatarMood;
  theme: AvatarTheme;
  small?: boolean;
  pixels: number;
  eventKey?: string;
  audioLevel?: SharedValue<number>;
  visible?: boolean;
}
export function AvatarStill({
  config,
  kind,
  mood,
  theme,
  small,
  pixels,
}: Props) {
  const scene = useMemo(
    () =>
      drawAvatar(config, {
        mood,
        theme,
        kind,
        size: small ? 16 : pixels,
        native: true,
        reduced: true,
      }),
    [config, kind, mood, theme, small, pixels],
  );
  return (
    <View
      pointerEvents="none"
      testID="agent-avatar-still"
      style={{ width: pixels, height: pixels }}
    >
      <AvatarSvg scene={scene} />
    </View>
  );
}
/** Rich textures stay cached and static on native. Quiet motion runs on the UI
 * thread; eye cutout frames switch discretely, never crossfade opaque holes. */
export function AvatarAnimated({
  config,
  kind,
  mood: observedMood = "idle",
  agentId,
  theme,
  pixels,
  eventKey,
  audioLevel,
  visible = true,
}: Props) {
  const reduced = useReducedMotion(),
    active = useAvatarAnimationActive() && visible,
    phase = useSharedValue(0);
  const candidate = useSyncExternalStore(
    subscribeCreatedAvatars,
    () => createdAvatarEvent(agentId),
    () => null,
  );
  const claimant = useRef(Symbol("focused-native-avatar"));
  const [created, setCreated] = useState<string | null>(null);
  const yielded = useRef<string | null>(null);
  useEffect(() => {
    if (!candidate) {
      setCreated(null);
      return;
    }
    const eligible = active && pixels >= 24;
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
  }, [candidate, active, pixels, observedMood, created]);
  const creation =
    observedMood === "idle" && created === candidate?.id ? created : null;
  const mood = creation ? "created" : observedMood;
  const processEventKey = `${mood}:${creation ?? eventKey ?? "state"}`;
  const reaction = reactionFor(config, mood, pixels, true, reduced);
  const signature = reaction.activityMotion;
  const oneShot = !signature && reaction.oneShot;
  const moving =
    active &&
    !reduced &&
    !reaction.still &&
    pixels >= 24 &&
    (signature
      ? signature.period !== null
      : !reaction.effective.endsWith("-pose"));
  const scene = useMemo(
    () =>
      drawAvatar(config, {
        mood,
        theme,
        kind,
        size: pixels,
        native: true,
        reduced,
      }),
    [config, kind, mood, theme, pixels, reduced],
  );
  // The ring fades independently of the moving face.
  const activityRing = scene.paths.find((path) => path.role === "activity-ring");
  const face = useMemo(
    () => (activityRing ? { ...scene, paths: scene.paths.filter((path) => path !== activityRing) } : scene),
    [activityRing, scene],
  );
  const blink =
    !signature &&
    moving &&
    scene.surface === "flat" &&
    reaction.effective === "blink";
  const eyeFrames = useMemo(
    () => ({
      open: face,
      closed: blink
        ? drawAvatar(config, {
            mood,
            theme,
            kind,
            size: pixels,
            native: true,
            eyeFrame: "closed",
          })
        : scene,
      left: blink
        ? drawAvatar(config, {
            mood,
            theme,
            kind,
            size: pixels,
            native: true,
            eyeFrame: "left",
          })
        : scene,
      right: blink
        ? drawAvatar(config, {
            mood,
            theme,
            kind,
            size: pixels,
            native: true,
            eyeFrame: "right",
          })
        : scene,
    }),
    [blink, config, face, kind, mood, theme, pixels, scene],
  );
  const progress = useRef({ key: processEventKey, at: 0 });
  useEffect(() => {
    cancelAnimation(phase);
    const key = processEventKey;
    if (progress.current.key !== key) {
      progress.current = { key, at: 0 };
      phase.value = 0;
    }
    if (!moving) return;
    if (oneShot && progress.current.at >= 1) return;
    phase.value = oneShot ? progress.current.at : 0;
    const motion = withTiming(1, {
      duration: oneShot
        ? reaction.duration * 1000 * (1 - progress.current.at)
        : (signature?.period ?? 6) * 1000,
      easing: Easing.linear,
    });
    phase.value = oneShot ? motion : withRepeat(motion, -1, false);
    return () => {
      progress.current.at = phase.value;
      cancelAnimation(phase);
    };
  }, [
    moving,
    oneShot,
    reaction.duration,
    phase,
    mood,
    processEventKey,
    signature,
  ]);
  const activitySample = useDerivedValue(() =>
    signature
      ? sampleActivityMotion(
          signature,
          moving ? phase.value * (signature.period ?? 0) : 0,
        )
      : null,
  );
  const effect = reaction.effective;
  const reactiveLevel =
    moving && effect === "output-pulse" ? audioLevel : undefined;
  const bodyStyle = useAnimatedStyle(() => {
    const activity = activitySample.value;
    if (activity) {
      const { x, y, rotate, scale } = activity.transform;
      return {
        transform: [
          { translateX: (x * pixels) / 512 },
          { translateY: (y * pixels) / 512 },
          { rotate: `${rotate}deg` },
          { scale },
        ],
      };
    }
    const value = quietMotion(
      effect,
      phase.value,
      reactiveLevel?.value ?? 0,
      pixels,
      moving,
      oneShot,
    );
    return { transform: [{ translateY: value.y }, { scale: value.scale }] };
  });
  const haloStyle = useAnimatedStyle(() => {
    const activity = activitySample.value;
    if (activity) {
      return {
        opacity: activity.ringOpacity,
        transform: [{ scale: 1 }, { rotate: "0deg" }],
      };
    }
    const value = quietMotion(
      effect,
      phase.value,
      reactiveLevel?.value ?? 0,
      pixels,
      moving,
      oneShot,
    );
    return {
      opacity: reaction.still ? 0.55 : value.haloOpacity,
      transform: [
        { scale: value.haloScale },
        { rotate: `${value.haloRotate}deg` },
      ],
    };
  });
  const openStyle = useAnimatedStyle(() => ({
    opacity: quietEyeFrame(phase.value, blink) === "open" ? 1 : 0,
  }));
  const closedStyle = useAnimatedStyle(() => ({
    opacity: quietEyeFrame(phase.value, blink) === "closed" ? 1 : 0,
  }));
  const leftStyle = useAnimatedStyle(() => ({
    opacity: quietEyeFrame(phase.value, blink) === "left" ? 1 : 0,
  }));
  const rightStyle = useAnimatedStyle(() => ({
    opacity: quietEyeFrame(phase.value, blink) === "right" ? 1 : 0,
  }));
  const accent =
    FACE_GEOMETRY.palettes[config.choices.palette ?? "core-nyx"]?.colors[
      theme === "light" ? 1 : 0
    ] ?? "#b48cff";
  const legacyHalo =
    !quietActivityMotion(reaction.state) &&
    reaction.state !== "idle" &&
    reaction.id !== "still";
  return (
    <View
      pointerEvents="none"
      testID={reduced ? "agent-avatar-static" : "agent-avatar-animated"}
      style={{ width: pixels, height: pixels }}
    >
      <Animated.View
        testID="avatar-motion-face"
        style={[StyleSheet.absoluteFill, bodyStyle]}
      >
        <Animated.View style={[StyleSheet.absoluteFill, openStyle]}>
          <AvatarSvg scene={eyeFrames.open} />
        </Animated.View>
        {blink ? (
          <>
            <Animated.View style={[StyleSheet.absoluteFill, closedStyle]}>
              <AvatarSvg scene={eyeFrames.closed} />
            </Animated.View>
            <Animated.View style={[StyleSheet.absoluteFill, leftStyle]}>
              <AvatarSvg scene={eyeFrames.left} />
            </Animated.View>
            <Animated.View style={[StyleSheet.absoluteFill, rightStyle]}>
              <AvatarSvg scene={eyeFrames.right} />
            </Animated.View>
          </>
        ) : null}
      </Animated.View>
      {activityRing || legacyHalo ? (
        <Animated.View
          testID={`avatar-accent-${activityRing ? reaction.state : effect}`}
          style={[StyleSheet.absoluteFill, haloStyle]}
        >
          <Svg
            width="100%"
            height="100%"
            viewBox="0 0 512 512"
            accessible={false}
          >
            {activityRing ? (
              <Path
                d={activityRing.d}
                fill={activityRing.fill}
                stroke={activityRing.stroke}
                strokeWidth={activityRing.width}
              />
            ) : (
              <Circle
                cx={256}
                cy={256}
                r={239}
                fill="none"
                stroke={accent}
                strokeWidth={effect === "attentive" ? 7 : 4}
                strokeDasharray={
                  effect === "working-accent"
                    ? "100 1402"
                    : effect.endsWith("-pose")
                      ? "16 12"
                      : undefined
                }
              />
            )}
          </Svg>
        </Animated.View>
      ) : null}
    </View>
  );
}
