import { useEffect, useRef } from "react";
import { Animated, StyleSheet, View } from "react-native";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";
import type { UploadStatus } from "./assistantUploadStatus";

/** Status text owns accessibility; this bottom edge only provides a quiet visual cue. */
export function AssistantUploadProgress({ status }: { status: UploadStatus }) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const opacity = useRef(new Animated.Value(0.55)).current;
  useEffect(() => {
    opacity.setValue(0.55);
    if (reducedMotion || status.loader !== "indeterminate") return;
    const pulse = Animated.loop(
      Animated.sequence([
        Animated.timing(opacity, {
          toValue: 0.9,
          duration: 900,
          useNativeDriver: true,
        }),
        Animated.timing(opacity, {
          toValue: 0.35,
          duration: 900,
          useNativeDriver: true,
        }),
      ]),
    );
    pulse.start();
    return () => pulse.stop();
  }, [opacity, reducedMotion, status.loader]);
  if (status.loader === "none") return null;
  return (
    <View
      pointerEvents="none"
      accessible={false}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
      style={[styles.track, { backgroundColor: colors.borderSoft }]}
    >
      <Animated.View
        style={[
          styles.bar,
          {
            backgroundColor: colors.primary,
            width: `${status.loader === "determinate" ? status.progress : 40}%`,
            opacity: status.loader === "determinate" ? 0.75 : opacity,
          },
        ]}
      />
    </View>
  );
}
const styles = StyleSheet.create({
  track: { position: "absolute", left: 0, right: 0, bottom: 0, height: 2 },
  bar: { height: 2, borderRadius: 1 },
});
