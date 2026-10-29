import { useLayoutEffect, useRef, type PropsWithChildren } from "react";
import { useIsFocused } from "@react-navigation/native";
import { View } from "react-native";
import { useTheme } from "../theme/ThemeContext";
import { WorkspaceInteractionSurface } from "../lib/accessibility/WorkspaceInteractionSurface";

/** A root must be laid out and painted before the slab starts its return trip. */
export function WorkspacePageSurface({
  children,
  identity,
  onPainted,
}: PropsWithChildren<{
  identity: string;
  onPainted: (identity: string) => void;
}>) {
  const { colors } = useTheme();
  const focused = useIsFocused();
  const laidOut = useRef(false);
  const frame = useRef<number | undefined>(undefined);
  const current = useRef({ identity, onPainted });
  current.current = { identity, onPainted };
  const cancel = () => {
    if (frame.current !== undefined) cancelAnimationFrame(frame.current);
    frame.current = undefined;
  };
  const afterPaint = () => {
    cancel();
    const expected = current.current.identity;
    frame.current = requestAnimationFrame(() => {
      frame.current = requestAnimationFrame(() => {
        frame.current = undefined;
        if (expected === current.current.identity)
          current.current.onPainted(expected);
      });
    });
  };
  useLayoutEffect(() => {
    if (laidOut.current && focused) afterPaint();
    return cancel;
  }, [identity, focused]);
  return (
    <WorkspaceInteractionSurface style={{ flex: 1 }}>
      <View
        collapsable={false}
        style={{ flex: 1, backgroundColor: colors.bg, opacity: 1 }}
        onLayout={() => {
          laidOut.current = true;
          if (focused) afterPaint();
        }}
      >
        {children}
      </View>
    </WorkspaceInteractionSurface>
  );
}
