import { useRef, useState } from "react";
import {
  ScrollView,
  StyleSheet,
  View,
  type ScrollViewProps,
} from "react-native";
import { LinearGradient } from "expo-linear-gradient";
import { scrollEdges, type ScrollEdges } from "./scrollEdges";

export function useScrollEdges() {
  const geometry = useRef({ viewport: 0, content: 0, offset: 0 });
  const [edges, setEdges] = useState<ScrollEdges>({ start: false, end: false });
  function update(values: Partial<typeof geometry.current>) {
    Object.assign(geometry.current, values);
    const { viewport, content, offset } = geometry.current;
    const next = scrollEdges(viewport, content, offset);
    setEdges((previous) =>
      previous.start === next.start && previous.end === next.end
        ? previous
        : next,
    );
  }
  return {
    edges,
    layout: (viewport: number) => update({ viewport }),
    content: (content: number) => update({ content }),
    scroll: (offset: number) => update({ offset }),
  };
}
export function ScrollEdgeFades({
  edges,
  color,
  horizontal = false,
}: {
  edges: ScrollEdges;
  color: string;
  horizontal?: boolean;
}) {
  return (
    <View
      pointerEvents="none"
      accessible={false}
      importantForAccessibility="no-hide-descendants"
      style={StyleSheet.absoluteFill}
    >
      {edges.start && (
        <LinearGradient
          pointerEvents="none"
          colors={[color, `${color}00`]}
          start={{ x: 0, y: 0 }}
          end={horizontal ? { x: 1, y: 0 } : { x: 0, y: 1 }}
          style={horizontal ? styles.left : styles.top}
        />
      )}
      {edges.end && (
        <LinearGradient
          pointerEvents="none"
          colors={[`${color}00`, color]}
          start={{ x: 0, y: 0 }}
          end={horizontal ? { x: 1, y: 0 } : { x: 0, y: 1 }}
          style={horizontal ? styles.right : styles.bottom}
        />
      )}
    </View>
  );
}
export function OverflowScrollView({
  children,
  fadeColor,
  ...props
}: ScrollViewProps & { fadeColor: string }) {
  const overflow = useScrollEdges();
  return (
    <View style={{ minWidth: 0, flexShrink: 1 }}>
      <ScrollView
        {...props}
        scrollEventThrottle={32}
        onLayout={(event) => {
          props.onLayout?.(event);
          overflow.layout(
            props.horizontal
              ? event.nativeEvent.layout.width
              : event.nativeEvent.layout.height,
          );
        }}
        onContentSizeChange={(width, height) => {
          props.onContentSizeChange?.(width, height);
          overflow.content(props.horizontal ? width : height);
        }}
        onScroll={(event) => {
          props.onScroll?.(event);
          overflow.scroll(
            props.horizontal
              ? event.nativeEvent.contentOffset.x
              : event.nativeEvent.contentOffset.y,
          );
        }}
      >
        {children}
      </ScrollView>
      <ScrollEdgeFades
        edges={overflow.edges}
        color={fadeColor}
        horizontal={!!props.horizontal}
      />
    </View>
  );
}
const styles = StyleSheet.create({
  top: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 10,
    opacity: 0.65,
  },
  bottom: { position: "absolute", bottom: 0, left: 0, right: 0, height: 22 },
  left: {
    position: "absolute",
    top: 0,
    bottom: 0,
    left: 0,
    width: 12,
    opacity: 0.65,
  },
  right: { position: "absolute", top: 0, bottom: 0, right: 0, width: 20 },
});
