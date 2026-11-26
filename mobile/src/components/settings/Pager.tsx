import { Pressable, StyleSheet, Text, View } from "react-native";
import { ChevronLeft, ChevronRight } from "lucide-react-native";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { useTheme } from "../../theme/ThemeContext";
import { typeScale } from "../../theme/designTokens";

/** Equal action columns keep page changes still while labels wrap with text size. */
export function Pager({
  page,
  total,
  disabled = false,
  onChange,
}: {
  page: number;
  total: number;
  disabled?: boolean;
  onChange: (page: number) => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const previousDisabled = disabled || page <= 1;
  const nextDisabled = disabled || page >= total;
  const previousColor = previousDisabled ? colors.textMuted : colors.primary;
  const nextColor = nextDisabled ? colors.textMuted : colors.primary;
  return (
    <View style={styles.row}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("Previous")}
        accessibilityHint={t("Shows the previous page of results")}
        accessibilityState={{ disabled: previousDisabled }}
        disabled={previousDisabled}
        onPress={() => {
          if (!previousDisabled) onChange(page - 1);
        }}
        style={({ pressed }) => [
          styles.action,
          pressed && !previousDisabled && { backgroundColor: colors.navActive },
        ]}
      >
        <ChevronLeft size={16} color={previousColor} accessible={false} />
        <Text style={[styles.label, { color: previousColor }]}>
          {t("Previous")}
        </Text>
      </Pressable>
      <Text
        accessibilityLabel={t("Page {{page}} of {{total}}", { page, total })}
        style={[styles.page, { color: colors.textSecondary }]}
      >
        {t("{{page}} / {{total}}", { page, total })}
      </Text>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={t("Next")}
        accessibilityHint={t("Shows the next page of results")}
        accessibilityState={{ disabled: nextDisabled }}
        disabled={nextDisabled}
        onPress={() => {
          if (!nextDisabled) onChange(page + 1);
        }}
        style={({ pressed }) => [
          styles.action,
          pressed && !nextDisabled && { backgroundColor: colors.navActive },
        ]}
      >
        <Text style={[styles.label, { color: nextColor }]}>{t("Next")}</Text>
        <ChevronRight size={16} color={nextColor} accessible={false} />
      </Pressable>
    </View>
  );
}
const styles = StyleSheet.create({
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    paddingHorizontal: 8,
  },
  action: {
    flex: 1,
    minWidth: 44,
    minHeight: 44,
    padding: 8,
    flexDirection: "row",
    gap: 4,
    alignItems: "center",
    justifyContent: "center",
  },
  label: { ...typeScale.body, textAlign: "center", flexShrink: 1 },
  page: {
    ...typeScale.body,
    textAlign: "center",
    minHeight: 44,
    minWidth: 44,
    flex: 1,
    paddingVertical: 12,
    paddingHorizontal: 4,
    flexShrink: 1,
    fontVariant: ["tabular-nums"],
  },
});
