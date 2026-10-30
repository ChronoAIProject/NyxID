import type { PropsWithChildren } from "react";
import { View, type StyleProp, type ViewStyle } from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { useWorkspaceAccessibility } from "./workspaceAccessibility";

/** Native transparent modals can retain a focused underlying navigator on Android. */
export function WorkspaceInteractionSurface({
  children,
  inSheet = false,
  style,
}: PropsWithChildren<{ inSheet?: boolean; style?: StyleProp<ViewStyle> }>) {
  const focused = useIsFocused();
  const { sheetOpen } = useWorkspaceAccessibility();
  const hidden = !focused || (sheetOpen && !inSheet);
  return (
    <View
      collapsable={false}
      accessible={false}
      style={style}
      accessibilityElementsHidden={hidden}
      importantForAccessibility={hidden ? "no-hide-descendants" : "auto"}
    >
      {children}
    </View>
  );
}
