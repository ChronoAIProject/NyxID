import { useEffect, useState, type PropsWithChildren, type Ref } from "react";
import { Animated, StyleSheet, View } from "react-native";
import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import { useTheme } from "../../theme/ThemeContext";
import { SkeletonRows } from "../../components/Skeleton";

/** The list measures and lands at latest underneath a stable transcript placeholder. */
export function TranscriptViewport({
  ready,
  children,
  viewportRef,
  onTouchStart,
}: PropsWithChildren<{
  ready: boolean;
  viewportRef?: Ref<View>;
  onTouchStart?: () => void;
}>) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  const [opacity] = useState(() => new Animated.Value(0));
  useEffect(() => {
    opacity.stopAnimation();
    if (!ready || reducedMotion) {
      opacity.setValue(ready ? 1 : 0);
      return;
    }
    const animation = Animated.timing(opacity, {
      toValue: 1,
      duration: 160,
      useNativeDriver: true,
    });
    animation.start();
    return () => animation.stop();
  }, [ready, reducedMotion, opacity]);
  return (
    <View style={styles.fill} onTouchStart={onTouchStart}>
      <Animated.View
        pointerEvents={ready ? "auto" : "none"}
        ref={viewportRef}
        collapsable={false}
        style={[styles.fill, { opacity }]}
        accessibilityElementsHidden={!ready}
        importantForAccessibility={ready ? "auto" : "no-hide-descendants"}
      >
        {children}
      </Animated.View>
      {!ready && (
        <View
          pointerEvents="none"
          style={[
            StyleSheet.absoluteFill,
            styles.placeholder,
            { backgroundColor: colors.bg },
          ]}
        >
          <SkeletonRows
            rows={3}
            icon={false}
            accessibilityLabel="Loading messages"
          />
        </View>
      )}
    </View>
  );
}
const styles = StyleSheet.create({
  fill: { flex: 1, minHeight: 0 },
  placeholder: { justifyContent: "flex-end", padding: 16 },
});
