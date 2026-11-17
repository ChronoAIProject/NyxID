import { Text, Platform } from "react-native";
import {
  FLOATING_HEADER_GAP,
  FLOATING_HEADER_MARGIN,
  WORKSPACE_HEADER_ROW,
  WORKSPACE_TITLE_MAX_FONT_SCALE,
  useWorkspaceHeaderSides,
} from "./workspaceHeaderMetrics";
import { useContext } from "react";
import { View, useWindowDimensions } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
import {
  workspaceStatusCluster,
  workspaceHeaderBand,
  workspaceHeaderRightPadding,
} from "./workspaceLayout";
import { WorkspaceCornerSensorContext, WorkspaceQuickRailContext, } from "./WorkspaceLayoutContext";

/** Navigator callbacks return a component; hooks belong to its render. */
export function WorkspacePageHeader(props: NativeStackHeaderProps) {
  return <WorkspacePageHeaderContent {...props} />;
}
function WorkspacePageHeaderContent({
  options,
  route,
  back,
}: NativeStackHeaderProps) {
  const { colors } = useTheme();
  const insets = useSafeAreaInsets();
  // The row sits at the top inset (flush at the top beside the Duo's status
  // column); see workspaceHeaderBand. Pages that keep the rail inset start
  // right under it.
  const window = useWindowDimensions();
  const cornerSensor = useContext(WorkspaceCornerSensorContext);
  // Rail mode or full screen: content starts right under the row.
  const rail = useContext(WorkspaceQuickRailContext);
  const band = workspaceHeaderBand(
    insets.top,
    cornerSensor,
    workspaceStatusCluster({ platform: Platform.OS, ...window, rightInset: insets.right, }),
    undefined,
    rail,
  );
  const sides = useWorkspaceHeaderSides();
  const margin = FLOATING_HEADER_MARGIN;

  const props = { canGoBack: !!back, tintColor: colors.textPrimary };
  const title = options.title ?? route.name;
  return (
    <View
      testID="workspace-page-header"
      style={{ ...band, backgroundColor: colors.bg }}
    >
      <View
        style={{
          ...WORKSPACE_HEADER_ROW,
          gap: FLOATING_HEADER_GAP,
          paddingLeft: margin + insets.left,
          paddingRight: workspaceHeaderRightPadding(insets.right, margin),
        }}
      >
        <View
          testID="workspace-header-leading"
          style={{ width: sides.width, alignItems: "flex-start" }}
        >
          <View
            onLayout={sides.leftLayout}
            style={{ flexDirection: "row", gap: FLOATING_HEADER_GAP }}
          >
            {options.headerLeft?.(props)}
          </View>
        </View>
        <View
          testID="workspace-header-title"
          style={{ flex: 1, minWidth: 0, alignItems: "center" }}
        >
          {typeof options.headerTitle === "function" ? (
            options.headerTitle({
              children: title,
              tintColor: colors.textPrimary,
            })
          ) : (
            <Text
              accessibilityRole="header"
              numberOfLines={1}
              maxFontSizeMultiplier={WORKSPACE_TITLE_MAX_FONT_SCALE}
              style={{
                color: colors.textPrimary,
                fontFamily: fonts.displaySemi,
                fontSize: 18,
                lineHeight: 24,
                textAlign: "center",
              }}
            >
              {typeof options.headerTitle === "string"
                ? options.headerTitle
                : title}
            </Text>
          )}
        </View>
        <View
          testID="workspace-header-trailing"
          style={{ width: sides.width, alignItems: "flex-end" }}
        >
          <View
            onLayout={sides.rightLayout}
            style={{ flexDirection: "row", gap: FLOATING_HEADER_GAP }}
          >
            {options.headerRight?.(props)}
          </View>
        </View>
      </View>
    </View>
  );
}
