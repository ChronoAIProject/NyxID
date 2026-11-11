import { useEffect, useRef, useState } from "react";
import { StyleSheet, View } from "react-native";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";

export type LevelSubscription = (
  listener: (level: number) => void,
) => () => void;
/** Shared native level line: newest sample trails, silence is a row of dots. */
export function DictationWaveform({
  subscribe,
  height = 22,
  barWidth = 2,
  barGap = 3,
  minimumHeight = 2,
}: {
  subscribe: LevelSubscription;
  height?: number;
  barWidth?: number;
  barGap?: number;
  minimumHeight?: number;
}) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const [width, setWidth] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const count = Math.max(0, Math.floor((width + barGap) / (barWidth + barGap)));
  const capacity = useRef(count);
  capacity.current = count;
  useEffect(() => {
    setLevels([]);
    if (reducedMotion) return;
    return subscribe((level) =>
      setLevels((previous) =>
        [...previous, Math.max(0, Math.min(1, level))].slice(
          -Math.max(1, capacity.current),
        ),
      ),
    );
  }, [subscribe, reducedMotion]);
  const samples = reducedMotion ? [] : levels;
  const bars = [
    ...Array<number>(Math.max(0, count - samples.length)).fill(0),
    ...samples.slice(-count),
  ];
  return (
    <View
      testID="dictation-waveform"
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      onLayout={(event) => setWidth(event.nativeEvent.layout.width)}
      style={[styles.wave, { height, gap: barGap }]}
    >
      {bars.map((level, index) => (
        <View
          key={index}
          style={[
            styles.bar,
            {
              width: barWidth,
              borderRadius: barWidth / 2,
              height: minimumHeight + level * (height - minimumHeight),
              backgroundColor: colors.textSecondary,
            },
          ]}
        />
      ))}
    </View>
  );
}
const styles = StyleSheet.create({
  wave: {
    flex: 1,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "flex-end",
    overflow: "hidden",
  },
  bar: { width: 2, borderRadius: 1 },
});
