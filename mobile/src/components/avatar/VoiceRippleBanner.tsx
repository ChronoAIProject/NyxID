import { useEffect, useMemo, useRef, useState } from "react";
import { View, StyleSheet } from "react-native";
import Svg, { Circle, Rect } from "react-native-svg";
import Animated, {
  cancelAnimation,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withTiming,
  Easing,
  type SharedValue,
} from "react-native-reanimated";
import { DynamicTypeText as Text } from "../DynamicTypeText";
import {
  drawAvatar,
  FACE_GEOMETRY,
} from "../../../../shared/agent-avatars/draw";
import {
  cachedVoiceRippleFrames,
  prepareVoiceRippleFrames,
  voiceRippleDots,
  type VoiceRippleFrames,
  voiceFace,
  VOICE_RIPPLE_FRAMES,
  VOICE_RIPPLE_PERIOD,
  claimVoiceRipple,
  releaseVoiceRipple,
  subscribeVoiceRipple,
} from "../../../../shared/agent-avatars/voice-ripple";
import type { AvatarConfig } from "../../lib/avatars/agentAvatars";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useAvatarAnimationActive } from "./useAvatarAnimationActive";
import { AvatarSvg } from "./AvatarSvg";
interface Props {
  config: AvatarConfig;
  kind?: string;
  name?: string;
  status: string;
  inputAccepting: boolean;
  outputPlaying: boolean;
  outputLevel?: SharedValue<number>;
  visible?: boolean;
}
export function VoiceRippleBanner({
  config,
  kind,
  name = "NyxBot",
  status,
  inputAccepting,
  outputPlaying,
  outputLevel,
  visible = true,
}: Props) {
  const reduced = useReducedMotion() || config.choices.motion === "still",
    focused = useAvatarAnimationActive(),
    phase = useSharedValue(3),
    owner = useRef(Symbol("voice-ripple")),
    [leased, setLeased] = useState(false);
  const palette = config.choices.ripple_palette ?? "nyx",
    p = FACE_GEOMETRY.ripples[palette] ?? FACE_GEOMETRY.ripples.nyx!,
    dots = useMemo(() => voiceRippleDots(palette), [palette]);
  const [prepared, setPrepared] = useState<VoiceRippleFrames | undefined>(() =>
    cachedVoiceRippleFrames(palette),
  );
  const frames = prepared?.palette === palette ? prepared : undefined;
  const eligible =
    visible && focused && !reduced && (inputAccepting || outputPlaying);
  useEffect(() => {
    if (!eligible) {
      setLeased(false);
      return;
    }
    const refresh = () => setLeased(claimVoiceRipple(owner.current));
    const off = subscribeVoiceRipple(refresh);
    refresh();
    return () => {
      off();
      releaseVoiceRipple(owner.current);
    };
  }, [eligible]);
  useEffect(() => {
    let live = true;
    const present = cachedVoiceRippleFrames(palette);
    setPrepared(present);
    if (eligible && leased && !present) {
      prepareVoiceRippleFrames(palette)
        .then((result) => {
          if (live) setPrepared(result);
        })
        .catch(() => {
          // A palette/visibility change cancels preparation; keep the complete poster.
        });
    }
    return () => {
      live = false;
    };
  }, [palette, eligible, leased]);
  useEffect(() => {
    cancelAnimation(phase);
    phase.value = 3;
    if (eligible && leased && frames) {
      phase.value = 0;
      phase.value = withRepeat(
        withTiming(VOICE_RIPPLE_FRAMES, {
          duration: VOICE_RIPPLE_PERIOD * 1000,
          easing: Easing.linear,
        }),
        -1,
        false,
      );
    }
    return () => cancelAnimation(phase);
  }, [eligible, leased, phase, frames]);
  const face = useMemo(
    () =>
      drawAvatar(voiceFace(config), {
        size: 112,
        kind,
        theme: p.light ? "light" : "dark",
        ambient: p.ring,
        resting: true,
        reduced: true,
      }),
    [config, kind, p],
  );
  const reactiveLevel =
    eligible && leased && outputPlaying ? outputLevel : undefined;
  const amplitude = useAnimatedStyle(() => ({
    opacity: reactiveLevel
      ? 0.55 + 0.45 * Math.max(0, Math.min(1, reactiveLevel.value))
      : 1,
  }));
  return (
    <View
      testID="voice-ripple-banner"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.banner, { backgroundColor: p.bg }]}
    >
      <Svg
        style={StyleSheet.absoluteFill}
        width="100%"
        height="100%"
        viewBox="0 0 384 128"
        preserveAspectRatio="none"
      >
        <Rect width={384} height={128} fill={p.bg} />
        {dots.map((dot, index) => (
          <Circle
            key={index}
            cx={dot.x}
            cy={dot.y}
            r={dot.r}
            fill={dot.color}
            opacity={dot.opacity}
          />
        ))}
      </Svg>
      <Animated.View style={[StyleSheet.absoluteFill, amplitude]}>
        {frames?.frames.map((uri, index) => (
          <RippleFrame key={index} uri={uri} index={index} phase={phase} />
        ))}
      </Animated.View>
      <View style={[styles.face, { backgroundColor: p.ring }]}>
        <AvatarSvg scene={face} />
      </View>
      <View style={styles.copy}>
        <Text
          maxFontSizeMultiplier={1.6}
          style={[styles.status, { color: p.text }]}
        >
          {status}
        </Text>
        <Text maxFontSizeMultiplier={1.6} style={{ color: p.text }}>
          {name}
        </Text>
      </View>
    </View>
  );
}
function RippleFrame({
  uri,
  index,
  phase,
}: {
  uri: string;
  index: number;
  phase: SharedValue<number>;
}) {
  const style = useAnimatedStyle(() => ({
    opacity:
      ((Math.floor(phase.value) % VOICE_RIPPLE_FRAMES) + VOICE_RIPPLE_FRAMES) %
        VOICE_RIPPLE_FRAMES ===
      index
        ? 1
        : 0,
  }));
  return (
    <Animated.Image
      testID={`voice-ripple-frame-${index}`}
      source={{ uri }}
      resizeMode="stretch"
      fadeDuration={0}
      style={[StyleSheet.absoluteFill, style]}
    />
  );
}
const styles = StyleSheet.create({
  banner: {
    width: "100%",
    maxWidth: 600,
    aspectRatio: 3,
    alignSelf: "center",
    minHeight: 112,
    overflow: "hidden",
    borderRadius: 24,
    flexDirection: "row",
    alignItems: "center",
  },
  face: {
    width: "24%",
    aspectRatio: 1,
    marginLeft: "5%",
    borderRadius: 999,
    overflow: "hidden",
  },
  copy: { flex: 1, marginLeft: "5%", paddingRight: 12, gap: 4 },
  status: { fontSize: 19, fontWeight: "600" },
});
