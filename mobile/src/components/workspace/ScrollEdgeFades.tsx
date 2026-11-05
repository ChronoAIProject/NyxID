import { useRef, useState, type Ref } from "react";
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
  page = false,
}: {
  edges: ScrollEdges;
  color: string;
  horizontal?: boolean;
  /** Full-page scrollers get a taller, softer fade than inline fields. */
  page?: boolean;
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
          style={horizontal ? styles.left : page ? styles.pageTop : styles.top}
        />
      )}
      {edges.end && (
        <LinearGradient
          pointerEvents="none"
          colors={[`${color}00`, color]}
          start={{ x: 0, y: 0 }}
          end={horizontal ? { x: 1, y: 0 } : { x: 0, y: 1 }}
          style={horizontal ? styles.right : page ? styles.pageBottom : styles.bottom}
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
/**
 * The scroller for whole pages and pull-up sheets: when content is clipped at
 * the top or bottom, a subtle fade into the page colour says there is more.
 * Pages get this by default through SettingsPage and WorkspacePage.
 */
export function PageScrollView({
  children,
  fadeColor,
  scrollRef,
  ...props
}: ScrollViewProps & { fadeColor: string; scrollRef?: Ref<ScrollView> }) {
  const overflow = useScrollEdges();
  return (
    <View style={styles.page}>
      <ScrollView
        ref={scrollRef}
        {...props}
        scrollEventThrottle={32}
        onLayout={(event) => {
          props.onLayout?.(event);
          overflow.layout(event.nativeEvent.layout.height);
        }}
        onContentSizeChange={(width, height) => {
          props.onContentSizeChange?.(width, height);
          overflow.content(height);
        }}
        onScroll={(event) => {
          props.onScroll?.(event);
          overflow.scroll(event.nativeEvent.contentOffset.y);
        }}
      >
        {children}
      </ScrollView>
      <ScrollEdgeFades edges={overflow.edges} color={fadeColor} page />
    </View>
  );
}

const styles = StyleSheet.create({
  page: { flex: 1, minHeight: 0 },
  pageTop: {
    position: "absolute",
    top: 0,
    left: 0,
    right: 0,
    height: 18,
    opacity: 0.8,
  },
  pageBottom: {
    position: "absolute",
    bottom: 0,
    left: 0,
    right: 0,
    height: 32,
  },
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
