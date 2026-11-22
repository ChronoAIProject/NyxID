import { useContext } from "react";
import type { ViewProps } from "react-native";
import Animated, {
  FadeIn,
  FadeOut,
  ReduceMotion,
  useAnimatedStyle,
} from "react-native-reanimated";
import { SLAB_MOTION_DURATION } from "./workspaceSlabTransitions";
import { WorkspaceRailMotionContext } from "./workspaceFloatingInset";

/** An animatable view for floating controls (static in tests). */
export const MotionView = Animated.View;

/** Icons that swap inside a floating control fade in and out. */
export const HEADER_ACTION_ENTERING = FadeIn.duration(
  SLAB_MOTION_DURATION,
).reduceMotion(ReduceMotion.System);
export const HEADER_ACTION_EXITING = FadeOut.duration(150).reduceMotion(
  ReduceMotion.System,
);

/**
 * A content container on the Duo (folded or unfolded): its trailing padding
 * is its own plus the rail inset, which animates to 0 on the UI thread as
 * full screen gives the rail column to the content (and back). Plain
 * elsewhere.
 */
export function RailInsetView({
  ownPaddingRight = 0,
  style,
  ...props
}: ViewProps & { ownPaddingRight?: number }) {
  const rail = useContext(WorkspaceRailMotionContext);
  const inset = rail?.inset ?? 0;
  const progress = rail?.progress;
  const animated = useAnimatedStyle(
    () => ({
      paddingRight:
        ownPaddingRight + inset * (1 - (progress ? progress.value : 0)),
    }),
    [ownPaddingRight, inset, progress],
  );
  return <Animated.View {...props} style={rail ? [style, animated] : style} />;
}

/**
 * The rail's grouped shortcuts: on the fallback surface they fade out with
 * full screen and back in with rail mode. Liquid glass hides itself instead
 * (`fadeOpacity` false): glass under a fading ancestor stops rendering.
 */
export function RailShortcutsView({
  style,
  fadeOpacity = true,
  ...props
}: ViewProps & { fadeOpacity?: boolean }) {
  const rail = useContext(WorkspaceRailMotionContext);
  const progress = rail?.progress;
  const animated = useAnimatedStyle(
    () => ({ opacity: 1 - (progress ? progress.value : 0) }),
    [progress],
  );
  return (
    <Animated.View {...props} style={fadeOpacity ? [style, animated] : style} />
  );
}

/**
 * An icon inside the grouped shortcuts capsule: it fades out with full screen
 * and back in with rail mode on the UI thread, with the content's width.
 * Never an ancestor of glass.
 */
export function RailShortcutIcon(props: ViewProps) {
  const rail = useContext(WorkspaceRailMotionContext);
  const progress = rail?.progress;
  const animated = useAnimatedStyle(
    () => ({ opacity: 1 - (progress ? progress.value : 0) }),
    [progress],
  );
  return <Animated.View {...props} style={[props.style, animated]} />;
}
