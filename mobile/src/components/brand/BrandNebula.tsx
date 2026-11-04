import { useEffect, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  cancelAnimation,
  useAnimatedStyle,
  useSharedValue,
  withRepeat,
  withTiming,
} from "react-native-reanimated";
import Svg, { Circle, Defs, RadialGradient, Stop } from "react-native-svg";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";

/**
 * The sign-in backdrop: slow violet light drifting behind the brand mark,
 * purple as identity rather than wallpaper. Decorative and hidden from
 * VoiceOver; Reduce Motion keeps it still.
 */
export function BrandNebula({ height = 420 }: { height?: number }) {
  const { mode } = useTheme();
  const reducedMotion = useReducedMotion();
  const drift = useSharedValue(0);
  useEffect(() => {
    cancelAnimation(drift);
    drift.value = reducedMotion
      ? 0
      : withRepeat(
          withTiming(1, { duration: 9000, easing: Easing.inOut(Easing.sin) }),
          -1,
          true,
        );
    return () => cancelAnimation(drift);
  }, [reducedMotion]);
  const dark = mode === "dark";
  const a = useAnimatedStyle(() => ({
    transform: [
      { translateX: -40 + drift.value * 60 },
      { translateY: -20 + drift.value * 24 },
      { scale: 1 + drift.value * 0.08 },
    ],
  }));
  const b = useAnimatedStyle(() => ({
    transform: [
      { translateX: 60 - drift.value * 70 },
      { translateY: 30 - drift.value * 30 },
      { scale: 1.08 - drift.value * 0.1 },
    ],
  }));
  return (
    <View
      pointerEvents="none"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.wrap, { height }]}
    >
      <Animated.View style={[styles.cloud, a]}>
        <Glow id="nebula-violet" color="#9775FA" peak={dark ? 0.42 : 0.22} size={340} />
      </Animated.View>
      <Animated.View style={[styles.cloud, b]}>
        <Glow id="nebula-deep" color="#5E00F5" peak={dark ? 0.34 : 0.14} size={300} />
      </Animated.View>
    </View>
  );
}

function Glow({ id, color, peak, size }: { id: string; color: string; peak: number; size: number }) {
  const r = size / 2;
  return (
    <Svg width={size} height={size}>
      <Defs>
        <RadialGradient id={id} cx="50%" cy="50%" r="50%">
          <Stop offset="0" stopColor={color} stopOpacity={peak} />
          <Stop offset="0.55" stopColor={color} stopOpacity={peak * 0.4} />
          <Stop offset="1" stopColor={color} stopOpacity={0} />
        </RadialGradient>
      </Defs>
      <Circle cx={r} cy={r} r={r} fill={`url(#${id})`} />
    </Svg>
  );
}

/**
 * The portal mark breathing gently on a soft violet glow; still under Reduce
 * Motion.
 */
export function BreathingMark({ children }: { children: ReactNode }) {
  const reducedMotion = useReducedMotion();
  const breath = useSharedValue(0);
  useEffect(() => {
    cancelAnimation(breath);
    breath.value = reducedMotion
      ? 0
      : withRepeat(
          withTiming(1, { duration: 2400, easing: Easing.inOut(Easing.sin) }),
          -1,
          true,
        );
    return () => cancelAnimation(breath);
  }, [reducedMotion]);
  const style = useAnimatedStyle(() => ({
    transform: [{ scale: 1 + breath.value * 0.035 }],
    shadowOpacity: 0.35 + breath.value * 0.25,
  }));
  return <Animated.View style={[styles.mark, style]}>{children}</Animated.View>;
}

const styles = StyleSheet.create({
  wrap: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    alignItems: "center",
    justifyContent: "center",
    overflow: "hidden",
  },
  cloud: { position: "absolute" },
  mark: {
    shadowColor: "#9775FA",
    shadowRadius: 28,
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.4,
  },
});
