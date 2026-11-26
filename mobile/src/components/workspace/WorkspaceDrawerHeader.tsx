import {
  FLOATING_HEADER_GAP,
  FLOATING_HEADER_MARGIN,
  WORKSPACE_HEADER_ROW,
  useWorkspaceHeaderSides,
} from "./workspaceHeaderMetrics";
import { useContext } from "react";
import { StyleSheet, Text, View } from "react-native";
import { WorkspaceSheetHeaderBandContext } from "./WorkspaceLayoutContext";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import { workspaceHeaderRightPadding } from "./workspaceLayout";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

/**
 * Settings and its account pages share centered, capped titles between the
 * same floating glass controls as every workspace bar (Back, a commit action
 * such as Save; only the sheet's own Close stays plain), 8pt apart with 12pt
 * side margins.
 */
export function WorkspaceDrawerHeader(props: NativeStackHeaderProps) {
  // This export is also the navigator's directly invoked `header` callback.
  return <WorkspaceDrawerHeaderContent {...props} />;
}

function WorkspaceDrawerHeaderContent({
  options,
  navigation,
  route,
}: NativeStackHeaderProps) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  const sides = useWorkspaceHeaderSides();
  const sheetIndex = navigation
    .getState()
    .routes.findIndex((item) => item.key === route.key);
  const props = { canGoBack: sheetIndex > 0, tintColor: colors.textPrimary };
  // Beside the Duo's cluster: flush under the sheet's handle, left of the
  // status column; the full-width body below starts under the cluster.
  const band = useContext(WorkspaceSheetHeaderBandContext);
  return (
    <View
      testID="workspace-sheet-header-band"
      style={{ ...band, backgroundColor: colors.bg }}
    >
      <View
        style={[
          styles.header,
          {
            paddingLeft: FLOATING_HEADER_MARGIN + insets.left,
            paddingRight: workspaceHeaderRightPadding(
              insets.right,
              FLOATING_HEADER_MARGIN,
            ),
          },
        ]}
      >
        <View
          testID="workspace-header-leading"
          style={{ width: sides.width, alignItems: "flex-start" }}
        >
          <View onLayout={sides.leftLayout} style={styles.controls}>
            {props.canGoBack ? options.headerLeft?.(props) : null}
          </View>
        </View>
        <Text
          accessibilityRole="header"
          numberOfLines={1}
          maxFontSizeMultiplier={1.25}
          style={[styles.title, { color: colors.textPrimary }]}
        >
          {options.title ?? route.name}
        </Text>
        <View
          testID="workspace-header-trailing"
          style={{ width: sides.width, alignItems: "flex-end" }}
        >
          <View onLayout={sides.rightLayout} style={styles.controls}>
            {options.headerRight?.(props)}
          </View>
        </View>
      </View>
    </View>
  );
}
const styles = StyleSheet.create({
  header: {
    ...WORKSPACE_HEADER_ROW,
    gap: FLOATING_HEADER_GAP,
  },
  controls: { flexDirection: "row", gap: FLOATING_HEADER_GAP },
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    flex: 1,
    minWidth: 0,
    textAlign: "center",
  },
});
