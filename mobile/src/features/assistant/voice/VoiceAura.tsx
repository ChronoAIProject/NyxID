import { useEffect } from "react";
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
import Svg, { Circle, Defs, RadialGradient, Stop } from "react-native-svg";
import { useTheme } from "../../../theme/ThemeContext";
import { voiceAuraTargets, type VoiceAuraPhase } from "./voiceAuraState";

type Props = {
  phase: VoiceAuraPhase;
  /** Microphone and playback loudness, 0..1, already eased on the UI thread. */
  input: SharedValue<number>;
  output: SharedValue<number>;
  reducedMotion: boolean;
  size: number;
};

/**
 * A soft mint-and-blush blob that shows what the call is doing. Decorative:
 * the status line carries the state for VoiceOver, so this is hidden from it.
 * Only transforms and opacity animate; the SVG gradients never re-render.
 */
export function VoiceAura({ phase, input, output, reducedMotion, size }: Props) {
  const { mode } = useTheme();
  const target = voiceAuraTargets(phase, reducedMotion);
  const breath = useSharedValue(0);
  const spin = useSharedValue(0);
  const opacity = useSharedValue(target.opacity);
  const scale = useSharedValue(target.scale);
  const gain = useSharedValue(target.gain);
  const orbit = useSharedValue(target.orbit);
  const breathing = useSharedValue(target.breathe ? 1 : 0);
  const fromOutput = useSharedValue(target.source === "output" ? 1 : 0);

  useEffect(() => {
    if (reducedMotion) {
      cancelAnimation(breath);
      cancelAnimation(spin);
      breath.value = 0;
      spin.value = 0;
      return;
    }
    breath.value = withRepeat(
      withTiming(1, { duration: 4200, easing: Easing.inOut(Easing.sin) }),
      -1,
      true,
    );
    // A full turn is seamless: positions are sines/cosines of whole multiples
    // of the angle (layer directions are integers).
    spin.value = withRepeat(
      withTiming(Math.PI * 2, { duration: 16000, easing: Easing.linear }),
      -1,
      false,
    );
    return () => {
      cancelAnimation(breath);
      cancelAnimation(spin);
    };
  }, [reducedMotion]);

  useEffect(() => {
    const ease = {
      duration: reducedMotion ? 240 : 650,
      easing: Easing.inOut(Easing.quad),
    };
    opacity.value = withTiming(target.opacity, ease);
    scale.value = withTiming(target.scale, ease);
    gain.value = withTiming(target.gain, ease);
    orbit.value = withTiming(target.orbit, { ...ease, duration: 900 });
    breathing.value = withTiming(target.breathe ? 1 : 0, ease);
    fromOutput.value = withTiming(target.source === "output" ? 1 : 0, {
      duration: 200,
    });
  }, [
    target.opacity,
    target.scale,
    target.gain,
    target.orbit,
    target.breathe,
    target.source,
    reducedMotion,
  ]);

  const values = { input, output, breath, spin, opacity, scale, gain, orbit, breathing, fromOutput };
  const glow = useAuraLayer(values, { swell: 1.1, breathe: 0.05, base: 1, glow: true });
  const first = useAuraLayer(values, { angle: 0, direction: 1, weight: 1 });
  const second = useAuraLayer(values, { angle: 2.1, direction: -1, weight: 1 });
  const third = useAuraLayer(values, { angle: 4.2, direction: 2, weight: 1.3 });
  const core = useAuraLayer(values, { swell: 0.6, breathe: 0.04, base: 0.94 });

  const dark = mode === "dark";
  // An organic blob: a mint body with a blush-and-lilac core that drift off
  // centre as they orbit, so the outline keeps changing shape.
  const layers = [
    {
      key: "glow",
      style: glow,
      diameter: size * 1.08,
      color: "#D6F7EC",
      peak: dark ? 0.25 : 0.45,
    },
    {
      key: "first",
      style: first,
      diameter: size * 0.92,
      color: "#B4F0DC",
      peak: dark ? 0.8 : 1,
    },
    {
      key: "second",
      style: second,
      diameter: size * 0.66,
      color: "#E3B9F4",
      peak: dark ? 0.85 : 0.95,
    },
    {
      key: "third",
      style: third,
      diameter: size * 0.52,
      color: "#F49FDA",
      peak: dark ? 0.9 : 1,
    },
    {
      key: "core",
      style: core,
      diameter: size * 0.42,
      color: "#FFFFFF",
      peak: dark ? 0.16 : 0.32,
    },
  ];
  return (
    <View
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      pointerEvents="none"
      style={{ width: size, height: size }}
    >
      {layers.map((layer) => (
        <Animated.View
          key={layer.key}
          style={[
            styles.layer,
            {
              width: layer.diameter,
              height: layer.diameter,
              left: (size - layer.diameter) / 2,
              top: (size - layer.diameter) / 2,
            },
            layer.style,
          ]}
        >
          <AuraBlob
            id={`aura-${layer.key}`}
            color={layer.color}
            peak={layer.peak}
            diameter={layer.diameter}
          />
        </Animated.View>
      ))}
    </View>
  );
}

type AuraValues = Record<
  | "input"
  | "output"
  | "breath"
  | "spin"
  | "opacity"
  | "scale"
  | "gain"
  | "orbit"
  | "breathing"
  | "fromOutput",
  SharedValue<number>
>;

/**
 * One layer's motion. Orbiting layers (`weight`) circle the centre at their
 * own angle and direction; centred layers (`swell`) only grow and glow.
 */
function useAuraLayer(
  v: AuraValues,
  layer: {
    angle?: number;
    direction?: number;
    weight?: number;
    swell?: number;
    breathe?: number;
    base?: number;
    glow?: boolean;
  },
) {
  const angle = layer.angle ?? 0;
  const direction = layer.direction ?? 0;
  const weight = layer.weight ?? 0;
  const swell = layer.swell ?? 0.8 * weight;
  const breatheBy = layer.breathe ?? 0.035 * weight;
  const base = layer.base ?? 1;
  const glow = layer.glow === true;
  return useAnimatedStyle(() => {
    const level =
      v.input.value * (1 - v.fromOutput.value) +
      v.output.value * v.fromOutput.value;
    const lift = v.gain.value * level;
    const breathe = v.breath.value * v.breathing.value;
    const turn = v.spin.value * direction + angle;
    return {
      opacity: Math.min(
        1,
        v.opacity.value * (glow ? 0.7 + lift * 1.2 : 0.85 + lift),
      ),
      transform: [
        { translateX: Math.cos(turn) * v.orbit.value * weight },
        { translateY: Math.sin(turn) * v.orbit.value * weight },
        { scale: v.scale.value * (base + breathe * breatheBy + lift * swell) },
      ],
    };
  });
}

/** One radial-gradient disc; gradient IDs are scoped to their own Svg root. */
function AuraBlob({
  id,
  color,
  peak,
  diameter,
}: {
  id: string;
  color: string;
  peak: number;
  diameter: number;
}) {
  const radius = diameter / 2;
  return (
    <Svg width={diameter} height={diameter}>
      <Defs>
        <RadialGradient id={id} cx="50%" cy="50%" r="50%">
          <Stop offset="0" stopColor={color} stopOpacity={peak} />
          <Stop offset="0.7" stopColor={color} stopOpacity={peak * 0.92} />
          <Stop offset="0.9" stopColor={color} stopOpacity={peak * 0.45} />
          <Stop offset="1" stopColor={color} stopOpacity={0} />
        </RadialGradient>
      </Defs>
      <Circle cx={radius} cy={radius} r={radius} fill={`url(#${id})`} />
    </Svg>
  );
}

const styles = StyleSheet.create({
  layer: { position: "absolute" },
});
