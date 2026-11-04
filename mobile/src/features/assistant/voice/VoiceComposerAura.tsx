import { useEffect, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  cancelAnimation,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withTiming,
  type SharedValue,
} from "react-native-reanimated";
import { LinearGradient } from "expo-linear-gradient";
import { useTheme } from "../../../theme/ThemeContext";
import type { VoiceAuraPhase } from "./voiceAuraState";

type Props = {
  phase: VoiceAuraPhase;
  /** Microphone and playback loudness, 0..1, already eased on the UI thread. */
  input: SharedValue<number>;
  output: SharedValue<number>;
  reducedMotion: boolean;
  children: ReactNode;
};

/** How bright the halo rests in each phase, and which loudness lifts it. */
const GLOW: Record<VoiceAuraPhase, { base: number; gain: number; source: "input" | "output"; pulse: boolean }> = {
  resting: { base: 0.18, gain: 0, source: "input", pulse: false },
  connecting: { base: 0.22, gain: 0, source: "input", pulse: true },
  listening: { base: 0.3, gain: 0.55, source: "input", pulse: false },
  thinking: { base: 0.3, gain: 0, source: "input", pulse: true },
  speaking: { base: 0.42, gain: 0.58, source: "output", pulse: false },
  muted: { base: 0.08, gain: 0, source: "input", pulse: false },
};

// The voice page's mint, lilac and blush, around the composer's edge.
const RING = ["#B4F0DC", "#E3B9F4", "#F49FDA", "#B4F0DC"] as const;

/**
 * The voice composer's aura: a soft pastel halo around its edge that brightens
 * and swells with NyxBot's voice when it speaks, follows your voice while it
 * listens and breathes while it thinks. Decorative and hidden from VoiceOver;
 * Reduce Motion keeps a still glow per phase.
 */
export function VoiceComposerAura({ phase, input, output, reducedMotion, children }: Props) {
  const { mode, colors } = useTheme();
  const glow = GLOW[phase];
  const base = useSharedValue(glow.base);
  const gain = useSharedValue(glow.gain);
  const fromOutput = useSharedValue(glow.source === "output" ? 1 : 0);
  const pulse = useSharedValue(0);
  useEffect(() => {
    const ease = { duration: reducedMotion ? 200 : 450, easing: Easing.inOut(Easing.quad) };
    base.value = withTiming(glow.base, ease);
    gain.value = withTiming(reducedMotion ? 0 : glow.gain, ease);
    fromOutput.value = withTiming(glow.source === "output" ? 1 : 0, { duration: 200 });
    cancelAnimation(pulse);
    pulse.value =
      glow.pulse && !reducedMotion
        ? withRepeat(withTiming(1, { duration: 1100, easing: Easing.inOut(Easing.sin) }), -1, true)
        : withTiming(0, { duration: 200 });
  }, [phase, reducedMotion]);
  const dark = mode === "dark";
  const level = () => {
    "worklet";
    return input.value * (1 - fromOutput.value) + output.value * fromOutput.value;
  };
  // Two soft coloured shadows, blush up-right and mint down-left, make the halo.
  const blush = useAnimatedStyle(() => {
    const lift = gain.value * level();
    return {
      shadowOpacity: Math.min(1, (base.value + pulse.value * 0.2 + lift) * (dark ? 1.4 : 1.1)),
      shadowRadius: 14 + lift * 16 + pulse.value * 4,
    };
  });
  const mint = useAnimatedStyle(() => {
    const lift = gain.value * level();
    return {
      shadowOpacity: Math.min(1, (base.value + pulse.value * 0.16 + lift * 0.8) * (dark ? 1.3 : 1)),
      shadowRadius: 12 + lift * 14 + pulse.value * 4,
    };
  });
  const edge = useAnimatedStyle(() => ({
    opacity: Math.min(1, base.value * 1.8 + pulse.value * 0.25 + gain.value * level() * 0.9),
  }));
  return (
    <View style={styles.wrap}>
      <View
        pointerEvents="none"
        accessible={false}
        accessibilityElementsHidden
        importantForAccessibility="no-hide-descendants"
        style={StyleSheet.absoluteFill}
      >
        <Animated.View
          style={[styles.glow, styles.blush, { backgroundColor: colors.composerBg }, blush]}
        />
        <Animated.View
          style={[styles.glow, styles.mint, { backgroundColor: colors.composerBg }, mint]}
        />
        <Animated.View style={[styles.edge, edge]}>
          <LinearGradient
            colors={RING}
            start={{ x: 0, y: 0 }}
            end={{ x: 1, y: 1 }}
            style={styles.fill}
          />
        </Animated.View>
      </View>
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  wrap: { position: "relative" },
  fill: { flex: 1 },
  glow: {
    ...StyleSheet.absoluteFillObject,
    borderRadius: 26,
    shadowOpacity: 0,
  },
  blush: { shadowColor: "#F49FDA", shadowOffset: { width: 6, height: -5 } },
  mint: { shadowColor: "#8FE6C8", shadowOffset: { width: -6, height: 5 } },
  edge: {
    position: "absolute",
    top: -1.5,
    left: -1.5,
    right: -1.5,
    bottom: -1.5,
    borderRadius: 27.5,
    overflow: "hidden",
  },
});
