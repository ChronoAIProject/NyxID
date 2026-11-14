import { ScrollView, StyleSheet } from "react-native";
import type { SheetScreenProps } from "../../app/navigationTypes";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { SettingsPage } from "../../components/settings/SettingsList";
import { terminalLicenses } from "../ssh/terminalAssets.generated";
import { reflowTerminalLicenses } from "./terminalLicenseText";
import { fonts, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";

export function OpenSourceNoticesScreen(
  _props: SheetScreenProps<"OpenSourceNotices">,
) {
  const { colors } = useTheme();
  return (
    <SettingsPage>
      <ScrollView
        style={styles.scroll}
        contentContainerStyle={styles.content}
        showsVerticalScrollIndicator
      >
        <Text selectable style={[styles.text, { color: colors.textPrimary }]}>
          {reflowTerminalLicenses(terminalLicenses).map(
            (notice, index, notices) => (
              <Text key={notice.packageName} style={styles.notice}>
                <Text style={styles.heading}>{notice.packageName}</Text>
                {"\n"}
                {notice.text}
                {index < notices.length - 1 ? "\n\n" : null}
              </Text>
            ),
          )}
        </Text>
      </ScrollView>
    </SettingsPage>
  );
}

const styles = StyleSheet.create({
  scroll: { flex: 1 },
  content: { padding: spacing.md, gap: spacing.md },
  text: { ...typeScale.mono, fontFamily: fonts.mono, lineHeight: 20 },
  notice: { ...typeScale.mono, fontFamily: fonts.mono, lineHeight: 20 },
  heading: { fontSize: 13, lineHeight: 18, fontWeight: "600" },
});
