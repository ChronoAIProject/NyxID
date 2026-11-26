import { ScrollView, StyleSheet, View } from "react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { SettingsPage } from "../../components/settings/SettingsList";
import { terminalLicenses } from "../ssh/terminalAssets.generated";
import { reflowTerminalLicenses } from "./terminalLicenseText";
import { fonts, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";

export function OpenSourceNoticesScreen(
  _props: SheetScreenProps<"OpenSourceNotices">,
) {
  const { colors } = useTheme();
  const railIndicator = useRailScrollIndicator();
  return (
    <SettingsPage>
      <ScrollView
        {...railIndicator}
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator
      >
        {reflowTerminalLicenses(terminalLicenses).map((notice) => (
          <View key={notice.packageName} style={styles.notice}>
            <Text
              accessibilityRole="header"
              style={[styles.heading, { color: colors.textPrimary }]}
            >
              {notice.packageName}
            </Text>
            <Text selectable style={[styles.text, { color: colors.textPrimary }]}>
              {notice.text}
            </Text>
          </View>
        ))}
      </ScrollView>
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  scroll: { flex: 1 },
  content: { padding: spacing.md, gap: spacing.md },
  text: { ...typeScale.mono, fontFamily: fonts.mono, lineHeight: 20 },
  notice: { gap: spacing.sm },
  heading: { ...typeScale.small, letterSpacing: 0.6 },
});
