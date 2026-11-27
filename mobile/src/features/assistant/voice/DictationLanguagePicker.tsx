import { useState } from "react";
import {
  Modal,
  Pressable,
  SectionList,
  StyleSheet,
  TextInput,
  View,
} from "react-native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { Check, X } from "lucide-react-native";
import { useAppLanguage } from "../../../i18n/LanguageProvider";
import { useTheme } from "../../../theme/ThemeContext";
import { fonts, radius, spacing, typeScale } from "../../../theme/designTokens";
import { DynamicTypeText as Text } from "../../../components/DynamicTypeText";
import { SettingsRow } from "../../../components/settings/SettingsList";
import { matchesLanguage } from "./dictationLanguageLabels";
import { manualDictationLocale } from "./recognitionLocales";
import type { useDictationLanguage } from "./useDictationLanguage";

type LanguageState = ReturnType<typeof useDictationLanguage>;
/** Full-screen transient choice; choosing changes only the next recording. */
export function DictationLanguagePicker({
  language,
  currentRecordingLanguage,
  onClose,
}: {
  language: LanguageState;
  currentRecordingLanguage?: string;
  onClose(): void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const [manual, setManual] = useState("");
  const [search, setSearch] = useState("");
  const automatic = language.options[0]!;
  const all = language.options
    .slice(1)
    .filter((row) => matchesLanguage(row, search));
  const suggested = language.suggested.flatMap((value) =>
    all.filter((row) => row.value === value),
  );
  const sections = [
    {
      key: "automatic",
      title: "",
      data: matchesLanguage(automatic, search) ? [automatic] : [],
    },
    {
      key: "suggested",
      title: t("Suggested"),
      data: suggested.map((row) => ({ ...row, key: `suggested:${row.value}` })),
    },
    {
      key: "all",
      title: t("All languages"),
      data: all.map((row) => ({ ...row, key: `all:${row.value}` })),
    },
  ].filter((section) => section.data.length);
  return (
    <Modal
      visible
      presentationStyle="fullScreen"
      animationType="slide"
      onRequestClose={onClose}
    >
      <SafeAreaProvider style={styles.page}>
        <SafeAreaView style={[styles.page, { backgroundColor: colors.bg }]}>
          <View style={styles.header}>
            <Text
              accessibilityRole="header"
              style={[styles.title, { color: colors.textPrimary }]}
            >
              {currentRecordingLanguage
                ? t("Language for the next recording")
                : t("Dictation language")}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={t("Close")}
              onPress={onClose}
              style={styles.close}
            >
              <X color={colors.textPrimary} size={22} />
            </Pressable>
          </View>
          {currentRecordingLanguage && (
            <Text
              style={[styles.currentLanguage, { color: colors.textSecondary }]}
            >
              {t("This recording continues in {{language}}.", {
                language: currentRecordingLanguage,
              })}
            </Text>
          )}
          <TextInput
            accessibilityLabel={t("Search languages")}
            placeholder={t("Search languages")}
            placeholderTextColor={colors.textMuted}
            value={search}
            onChangeText={setSearch}
            autoCorrect={false}
            style={[
              styles.search,
              {
                color: colors.textPrimary,
                backgroundColor: colors.groupedFill,
              },
            ]}
          />
          <SectionList
            sections={sections}
            keyExtractor={(item) =>
              "key" in item ? String(item.key) : `automatic:${item.value}`
            }
            keyboardShouldPersistTaps="handled"
            renderSectionHeader={({ section }) =>
              section.title ? (
                <Text
                  style={[
                    styles.section,
                    { color: colors.textMuted, backgroundColor: colors.bg },
                  ]}
                >
                  {section.title}
                </Text>
              ) : null
            }
            renderItem={({ item }) => (
              <SettingsRow
                label={item.label}
                detail={item.detail}
                chevron={false}
                accessibilityRole="radio"
                accessibilityState={{ checked: item.value === language.value }}
                accessory={
                  item.value === language.value ? (
                    <Check size={20} color={colors.textPrimary} />
                  ) : undefined
                }
                onPress={() => {
                  language.choose(item.value);
                  onClose();
                }}
              />
            )}
            ListEmptyComponent={
              <Text style={[styles.section, { color: colors.textSecondary }]}>
                {t("No languages found")}
              </Text>
            }
            ListFooterComponent={
              <View>
                <Text style={[styles.section, { color: colors.textMuted }]}>
                  {language.explanation ??
                    t("Changes apply to the next recording.")}
                </Text>
                {language.manualEntry && (
                  <View style={styles.recovery}>
                    <SettingsRow
                      label={t("Retry language list")}
                      onPress={language.retry}
                    />
                    <SettingsRow
                      label={t("Speech recognition settings")}
                      onPress={language.openSpeechSettings}
                    />
                    <TextInput
                      accessibilityLabel={t("Language code")}
                      placeholder={t("Language code (for example zh-CN)")}
                      value={manual}
                      onChangeText={setManual}
                      autoCapitalize="none"
                      autoCorrect={false}
                      style={[
                        styles.manual,
                        {
                          color: colors.textPrimary,
                          backgroundColor: colors.groupedFill,
                        },
                      ]}
                    />
                    <SettingsRow
                      label={t("Use language code")}
                      chevron={false}
                      disabled={!manualDictationLocale(manual)}
                      onPress={() => {
                        const locale = manualDictationLocale(manual);
                        if (locale) {
                          language.choose(locale);
                          onClose();
                        }
                      }}
                    />
                  </View>
                )}
              </View>
            }
          />
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}
const styles = StyleSheet.create({
  page: { flex: 1 },
  recovery: { paddingHorizontal: spacing.xxl, paddingBottom: spacing.xxl },
  manual: { padding: spacing.lg, borderRadius: radius.md, fontSize: 16 },
  header: {
    flexDirection: "row",
    alignItems: "center",
    paddingHorizontal: spacing.xxl,
  },
  title: { flex: 1, ...typeScale.pageHeader },
  currentLanguage: { paddingHorizontal: spacing.xxl, ...typeScale.caption },
  close: {
    minWidth: 44,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  search: {
    margin: spacing.xxl,
    padding: spacing.lg,
    borderRadius: radius.md,
    fontFamily: fonts.body,
    fontSize: 16,
  },
  section: { padding: spacing.xxl, ...typeScale.caption },
});
