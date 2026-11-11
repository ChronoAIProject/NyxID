import {
  WORKSPACE_HEADER_ROW,
  useWorkspaceHeaderSides,
} from "./workspaceHeaderMetrics";
import { Text, View } from "react-native";
import { useSafeAreaInsets } from "react-native-safe-area-context";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

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
  const sides = useWorkspaceHeaderSides();
  const props = { canGoBack: !!back, tintColor: colors.textPrimary };
  const title = options.title ?? route.name;
  return (
    <View
      testID="workspace-page-header"
      style={{ paddingTop: insets.top, backgroundColor: colors.bg }}
    >
      <View
        style={{
          ...WORKSPACE_HEADER_ROW,
          gap: 4,
          paddingLeft: 8,
          paddingRight: 8,
        }}
      >
        <View
          testID="workspace-header-leading"
          style={{ width: sides.width, alignItems: "flex-start" }}
        >
          <View onLayout={sides.leftLayout} style={{ flexDirection: "row" }}>
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
              maxFontSizeMultiplier={1.25}
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
          <View onLayout={sides.rightLayout} style={{ flexDirection: "row" }}>
            {options.headerRight?.(props)}
          </View>
        </View>
      </View>
    </View>
  );
}
