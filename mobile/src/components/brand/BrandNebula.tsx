import { useEffect, type ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import Animated, {
  Easing,
  useAnimatedStyle,
  useSharedValue,
  withDelay,
  withTiming,
} from "react-native-reanimated";
import Svg, { Circle, Defs, RadialGradient, Stop } from "react-native-svg";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";

/**
 * A faint, still halo behind the brand mark on sign-in and the splash: just
 * enough light to lift the mark off the page. No motion, no decoration.
 */
export function BrandNebula({
  height = 360,
  fill = false,
}: {
  height?: number;
  /** Centre the halo in the whole parent instead of its top band. */
  fill?: boolean;
}) {
  const { mode } = useTheme();
  const peak = mode === "dark" ? 0.16 : 0.07;
  const size = 300;
  const r = size / 2;
  return (
    <View
      pointerEvents="none"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.wrap, fill ? styles.fill : { height }]}
    >
      <Svg width={size} height={size}>
        <Defs>
          <RadialGradient id="brand-halo" cx="50%" cy="50%" r="50%">
            <Stop offset="0" stopColor="#9775FA" stopOpacity={peak} />
            <Stop offset="1" stopColor="#9775FA" stopOpacity={0} />
          </RadialGradient>
        </Defs>
        <Circle cx={r} cy={r} r={r} fill="url(#brand-halo)" />
      </Svg>
    </View>
  );
}

/** The brand mark with a soft neutral shadow for depth. */
export function BreathingMark({ children }: { children: ReactNode }) {
  return <View style={styles.mark}>{children}</View>;
}

/** Content that rises gently into place on arrival; immediate under Reduce Motion. */
export function RiseIn({ delay = 0, children }: { delay?: number; children: ReactNode }) {
  const reducedMotion = useReducedMotion();
  const shown = useSharedValue(reducedMotion ? 1 : 0);
  useEffect(() => {
    shown.value = reducedMotion
      ? 1
      : withDelay(delay, withTiming(1, { duration: 480, easing: Easing.out(Easing.cubic) }));
  }, []);
  const style = useAnimatedStyle(() => ({
    opacity: shown.value,
    transform: [{ translateY: (1 - shown.value) * 12 }],
  }));
  return <Animated.View style={style}>{children}</Animated.View>;
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
  fill: { bottom: 0 },
  mark: {
    shadowColor: "#000000",
    shadowRadius: 16,
    shadowOffset: { width: 0, height: 8 },
    shadowOpacity: 0.28,
  },
});
