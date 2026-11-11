import {
  WORKSPACE_HEADER_ROW,
  useWorkspaceHeaderSides,
} from "./workspaceHeaderMetrics";
import { StyleSheet, Text, View } from "react-native";
import type { NativeStackHeaderProps } from "@react-navigation/native-stack";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

/** Settings and its account pages share centered, capped titles. */
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
  const sides = useWorkspaceHeaderSides();
  const sheetIndex = navigation
    .getState()
    .routes.findIndex((item) => item.key === route.key);
  const props = { canGoBack: sheetIndex > 0, tintColor: colors.textPrimary };
  return (
    <View style={[styles.header, { backgroundColor: colors.bg }]}>
      <View
        testID="workspace-header-leading"
        style={{ width: sides.width, alignItems: "flex-start" }}
      >
        <View onLayout={sides.leftLayout} style={{ flexDirection: "row" }}>
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
        <View onLayout={sides.rightLayout} style={{ flexDirection: "row" }}>
          {options.headerRight?.(props)}
        </View>
      </View>
    </View>
  );
}
const styles = StyleSheet.create({
  header: {
    ...WORKSPACE_HEADER_ROW,
    gap: 4,
    paddingLeft: 12,
    paddingRight: 12,
  },
  title: {
    fontFamily: fonts.displaySemi,
    fontSize: 18,
    lineHeight: 24,
    flex: 1,
    minWidth: 0,
    textAlign: "center",
  },
});
