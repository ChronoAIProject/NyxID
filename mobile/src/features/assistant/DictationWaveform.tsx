import { useEffect, useRef, useState } from "react";
import { StyleSheet, View } from "react-native";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";

export type LevelSubscription = (
  listener: (level: number) => void,
) => () => void;
const BAR_PITCH = 5;

/** Shared native level line: newest sample trails, silence is a row of dots. */
export function DictationWaveform({
  subscribe,
  height = 22,
}: {
  subscribe: LevelSubscription;
  height?: number;
}) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const [width, setWidth] = useState(0);
  const [levels, setLevels] = useState<number[]>([]);
  const count = Math.max(0, Math.floor(width / BAR_PITCH));
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
      style={[styles.wave, { height }]}
    >
      {bars.map((level, index) => (
        <View
          key={index}
          style={[
            styles.bar,
            {
              height: 3 + level * (height - 3),
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
    gap: BAR_PITCH - 3,
    overflow: "hidden",
  },
  bar: { width: 3, borderRadius: 1.5 },
});
