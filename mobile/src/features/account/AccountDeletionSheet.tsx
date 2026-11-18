import { Modal, Pressable, ScrollView, StyleSheet, View } from "react-native";
import { DynamicTypeText as Text } from "../../components/DynamicTypeText";
import { ACCOUNT_DELETION_DISCLOSURE } from "../../lib/accountDeletionPolicy";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, radius, spacing, typeScale } from "../../theme/designTokens";

/**
 * Full-width confirmation for Delete Account. The system alert is too narrow
 * for the disclosure, so this page sheet lays it out as readable paragraphs
 * with the irreversible action last.
 */
export function AccountDeletionSheet({
  visible,
  onCancel,
  onConfirm,
}: {
  visible: boolean;
  onCancel: () => void;
  onConfirm: () => void;
}) {
  const { colors } = useTheme();
  const paragraphs = ACCOUNT_DELETION_DISCLOSURE.split("\n\n");
  return (
    <Modal
      visible={visible}
      animationType="slide"
      presentationStyle="pageSheet"
      onRequestClose={onCancel}
    >
      <View style={[styles.sheet, { backgroundColor: colors.bg }]}>
        <View style={styles.bar}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Cancel"
            hitSlop={12}
            onPress={onCancel}
            style={styles.barButton}
          >
            <Text style={[styles.barLabel, { color: colors.primary }]}>
              Cancel
            </Text>
          </Pressable>
        </View>
        <ScrollView
          style={styles.scroll}
          contentContainerStyle={styles.content}
          accessibilityLabel="Account deletion details"
        >
          <Text
            accessibilityRole="header"
            style={[styles.title, { color: colors.textPrimary }]}
          >
            Delete Account?
          </Text>
          <Text style={[styles.lead, { color: colors.danger }]}>
            This cannot be undone.
          </Text>
          <View style={[styles.card, { backgroundColor: colors.card }]}>
            {paragraphs.map((paragraph, index) => (
              <Text
                key={index}
                style={[styles.paragraph, { color: colors.textSecondary }]}
              >
                {paragraph}
              </Text>
            ))}
          </View>
        </ScrollView>
        <View style={[styles.actions, { borderTopColor: colors.borderSoft }]}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Delete Account permanently"
            onPress={onConfirm}
            style={({ pressed }) => [
              styles.delete,
              {
                backgroundColor: pressed
                  ? colors.dangerSolidPressed
                  : colors.dangerSolid,
              },
            ]}
          >
            <Text style={[styles.deleteLabel, { color: colors.onPrimary }]}>
              Delete Account
            </Text>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Keep my account"
            onPress={onCancel}
            style={styles.keep}
          >
            <Text style={[styles.keepLabel, { color: colors.primary }]}>
              Keep My Account
            </Text>
          </Pressable>
        </View>
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  sheet: { flex: 1 },
  bar: {
    flexDirection: "row",
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.lg,
    minHeight: 52,
    alignItems: "center",
  },
  barButton: { minHeight: 44, justifyContent: "center" },
  barLabel: { ...typeScale.label, fontSize: 16, lineHeight: 22 },
  scroll: { flex: 1 },
  content: {
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.huge,
    gap: spacing.lg,
  },
  title: { fontFamily: fonts.displaySemi, fontSize: 26, lineHeight: 32 },
  lead: { ...typeScale.label, fontSize: 15, lineHeight: 20 },
  card: {
    borderRadius: radius.lg,
    padding: spacing.xxl,
    gap: spacing.xxl,
  },
  paragraph: { ...typeScale.description, fontSize: 15, lineHeight: 22 },
  actions: {
    borderTopWidth: StyleSheet.hairlineWidth,
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.lg,
    paddingBottom: spacing.huge + spacing.lg,
    gap: spacing.sm,
  },
  delete: {
    minHeight: 50,
    borderRadius: radius.lg,
    alignItems: "center",
    justifyContent: "center",
  },
  deleteLabel: { ...typeScale.label, fontFamily: fonts.bodySemi, fontSize: 16, lineHeight: 22 },
  keep: { minHeight: 44, alignItems: "center", justifyContent: "center" },
  keepLabel: { ...typeScale.label, fontSize: 16, lineHeight: 22 },
});
