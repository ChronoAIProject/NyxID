import type { ReactNode, Ref } from "react";
import {
  Pressable,
  StyleSheet,
  View,
  type PressableProps,
  type StyleProp,
  type ViewProps,
  type ViewStyle,
} from "react-native";
import {
  GlassContainer,
  GlassView,
  isLiquidGlassAvailable,
} from "expo-glass-effect";
import { useTheme } from "../theme/ThemeContext";
import {
  floatingControlGlassTint,
  floatingControlSurface,
  type FloatingGlass,
} from "./workspace/workspaceFloatingSurface";
import {
  FLOATING_CONTROL_RADIUS,
  FLOATING_CONTROL_SIZE,
} from "./floatingControlTokens";

const GLASS_FADE_SECONDS = 0.25;

/*
 * The one style source for every floating control: header buttons, the
 * identity pill, the Duo's rail toggle and grouped capsule. On iOS 26+
 * each is Apple's liquid glass with the same explicit theme tint
 * (floatingControlGlassTint), style 'regular', interactive, in the app's (not
 * the system's) colour scheme, so the header and the rail sample the same
 * colour over the page. Without liquid glass (older iOS, Android) it is the
 * theme's opaque floating-control surface with the same token. Nothing else
 * draws a floating fill, border, shadow or glass.
 */

/** Whether floating controls render as native liquid glass here. */
export function floatingGlassAvailable() {
  return isLiquidGlassAvailable();
}

/**
 * The capsule's own colour, for marks inside it that cut out against it (an
 * avatar stack's ring); the primitive stays its only source.
 */
export function useFloatingSurfaceColor() {
  return useTheme().colors.floatingControlBg;
}

/**
 * One capsule holding its controls (FloatingItem), like grouped toolbar
 * buttons: a 44pt-wide column of targets, or a single control.
 *
 * Glass must never sit under a translucent or fading ancestor (it stops
 * rendering), so a capsule hides itself: `visible` animates the glass
 * material out natively (the fallback hides with its ancestor's fade).
 */
export function FloatingCapsule({
  radius = FLOATING_CONTROL_RADIUS,
  visible = true,
  glass = "standard",
  style,
  children,
  ...props
}: ViewProps & {
  radius?: number;
  visible?: boolean;
  /** The Duo rail's glass variants; every other control is `standard`. */
  glass?: FloatingGlass;
  children: ReactNode;
}) {
  const { colors, mode } = useTheme();
  if (isLiquidGlassAvailable())
    return (
      <GlassView
        {...props}
        isInteractive
        glassEffectStyle={{
          style: visible ? (glass === "rail" ? "clear" : "regular") : "none",
          animate: true,
          animationDuration: GLASS_FADE_SECONDS,
        }}
        // A hidden capsule's effect is 'none', but its tint would still
        // draw: clear it too, so nothing of a hidden capsule shows.
        tintColor={
          visible
            ? floatingControlGlassTint(colors, glass)
            : `${colors.floatingControlBg}00`
        }
        colorScheme={mode}
        style={[{ borderRadius: radius }, style]}
      >
        {children}
      </GlassView>
    );
  return (
    <View
      {...props}
      style={[
        floatingControlSurface(colors, mode, StyleSheet.hairlineWidth),
        { borderRadius: radius },
        style,
      ]}
    >
      {children}
    </View>
  );
}

/**
 * Capsules that belong together (the rail's toggle and grouped capsule): one
 * native glass container on iOS 26+, like Apple's grouped glass (shared
 * sampling; capsules closer than `spacing` morph together); a plain view
 * elsewhere.
 */
export function FloatingGroup({
  spacing,
  children,
  ...props
}: ViewProps & { spacing: number; children: ReactNode }) {
  return isLiquidGlassAvailable() ? (
    <GlassContainer {...props} spacing={spacing}>
      {children}
    </GlassContainer>
  ) : (
    <View {...props}>{children}</View>
  );
}

type FloatingItemProps = Omit<PressableProps, "style" | "children"> & {
  /** Layout of the control's contents (a pill's row and padding). */
  style?: StyleProp<ViewStyle>;
  children: ReactNode;
  ref?: Ref<View>;
};

/**
 * One control inside a floating capsule: a 44pt target with no surface of its
 * own. Pressing tints a highlight shaped like the capsule (round, or a pill;
 * iOS 26 glass also responds natively); a disabled control dims its
 * contents, never the capsule.
 */
export function FloatingItem({
  style,
  disabled,
  children,
  ref,
  ...props
}: FloatingItemProps) {
  const { colors } = useTheme();
  return (
    <Pressable
      ref={ref}
      accessibilityRole="button"
      accessibilityState={{ disabled: !!disabled }}
      disabled={disabled}
      {...props}
      style={({ pressed }) => [
        styles.item,
        style,
        pressed && !disabled ? { backgroundColor: colors.segmentTrack } : null,
        disabled ? styles.disabled : null,
      ]}
    >
      {children}
    </Pressable>
  );
}

/**
 * A floating button: one control in its own capsule (a 44pt circle, or a pill
 * that hugs its contents). The header's buttons and identity pill and the
 * rail's full-screen toggle all render through this.
 */
export function FloatingButton({
  capsuleStyle,
  capsuleGlass,
  ...props
}: FloatingItemProps & {
  capsuleStyle?: StyleProp<ViewStyle>;
  capsuleGlass?: FloatingGlass;
}) {
  return (
    <FloatingCapsule
      style={[styles.button, capsuleStyle]}
      glass={capsuleGlass}
    >
      <FloatingItem {...props} />
    </FloatingCapsule>
  );
}

const styles = StyleSheet.create({
  button: { minWidth: FLOATING_CONTROL_SIZE, minHeight: FLOATING_CONTROL_SIZE },
  item: {
    minWidth: FLOATING_CONTROL_SIZE,
    minHeight: FLOATING_CONTROL_SIZE,
    borderRadius: FLOATING_CONTROL_RADIUS,
    alignItems: "center",
    justifyContent: "center",
  },
  disabled: { opacity: 0.4 },
});
