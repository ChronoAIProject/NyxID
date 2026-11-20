import { Modal, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { SafeAreaView } from "react-native-safe-area-context";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { useNativeSheetDismiss } from "../../components/workspace/useNativeSheetDismiss";
import { useTheme } from "../../theme/ThemeContext";
import { radius, spacing, typeScale } from "../../theme/designTokens";

export type AttachmentSourceChoice = { label: string; onPress: () => void };

/** Android's three-button Alert stacks large text against one edge. Keep all
 * sources in a bounded vertical list and dismiss before opening a native picker. */
export function AttachmentSourcePicker({
  choices,
  onClose,
}: {
  choices: readonly AttachmentSourceChoice[];
  onClose: () => void;
}) {
  const { t } = useAppLanguage();
  const { colors } = useTheme();
  const sheet = useNativeSheetDismiss(onClose);
  return (
    <Modal {...sheet.modalProps} transparent onRequestClose={() => sheet.close()}>
      <SafeAreaView style={styles.backdrop}>
        <View style={[styles.panel, { backgroundColor: colors.card }]}>
          <ScrollView
            contentContainerStyle={styles.content}
            keyboardShouldPersistTaps="handled"
          >
            <Text
              accessibilityRole="header"
              style={[typeScale.title, { color: colors.textPrimary }]}
            >
              {t("Add attachments")}
            </Text>
            {choices.map((choice) => (
              <Pressable
                key={choice.label}
                accessibilityRole="button"
                onPress={() => sheet.close(choice.onPress)}
                style={styles.choice}
              >
                <Text style={[typeScale.label, { color: colors.textPrimary }]}>
                  {choice.label}
                </Text>
              </Pressable>
            ))}
            <Pressable
              accessibilityRole="button"
              onPress={() => sheet.close()}
              style={styles.choice}
            >
              <Text style={[typeScale.label, { color: colors.primary }]}>
                {t("Cancel")}
              </Text>
            </Pressable>
          </ScrollView>
        </View>
      </SafeAreaView>
    </Modal>
  );
}
const styles = StyleSheet.create({
  backdrop: {
    flex: 1,
    backgroundColor: "#00000066",
    justifyContent: "center",
    alignItems: "center",
    padding: spacing.xxl,
  },
  panel: {
    width: "100%",
    maxWidth: 480,
    maxHeight: "85%",
    borderRadius: radius.lg,
    overflow: "hidden",
  },
  content: { padding: spacing.xxl, gap: spacing.sm },
  choice: {
    minHeight: 48,
    paddingVertical: spacing.md,
    justifyContent: "center",
    alignSelf: "stretch",
  },
});
