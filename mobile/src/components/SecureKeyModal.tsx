import { useMemo, useState } from "react";
import { Modal, Pressable, StyleSheet, Text, TextInput, View } from "react-native";
import { ShieldCheck, Lock } from "lucide-react-native";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { TOUCH_TARGET, radius, spacing, typeScale } from "../theme/designTokens";
import type { SecretInputRequest } from "../lib/api/chatTypes";
import { PrimaryButton } from "./PrimaryButton";

type SecureKeyModalProps = {
  visible: boolean;
  fields: SecretInputRequest[];
  onSubmit: (values: { param_name: string; value: string }[]) => void;
  onCancel: () => void;
};

export function SecureKeyModal({ visible, fields, onSubmit, onCancel }: SecureKeyModalProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const [values, setValues] = useState<Record<string, string>>({});

  const handleSubmit = () => {
    onSubmit(fields.map((f) => ({ param_name: f.param_name, value: values[f.param_name] ?? "" })));
    setValues({});
  };

  const handleCancel = () => {
    setValues({});
    onCancel();
  };

  const serviceLabel = fields[0]?.label ?? "Service";

  return (
    <Modal visible={visible} transparent animationType="fade" onRequestClose={handleCancel}>
      <View style={styles.backdrop}>
        <View style={styles.modal}>
          <View style={styles.iconWrap}>
            <ShieldCheck size={18} color={colors.successTone.text} strokeWidth={2} />
          </View>
          <Text style={styles.title}>Secure API Key Input</Text>
          <Text style={styles.desc}>
            This key is stored directly in encrypted storage. It is{" "}
            <Text style={styles.descBold}>never sent to the LLM</Text> or processed by Nyx.
          </Text>

          <View style={styles.trustBadge}>
            <Lock size={10} color={colors.successTone.text} strokeWidth={2} />
            <Text style={styles.trustText}>End-to-end encrypted · LLM-isolated</Text>
          </View>

          <Text style={styles.fieldLabel}>SERVICE</Text>
          <View style={styles.serviceRow}>
            <View style={styles.serviceDot}>
              <Text style={styles.serviceDotText}>◆</Text>
            </View>
            <Text style={styles.serviceText}>{serviceLabel}</Text>
          </View>

          {fields.map((field) => (
            <View key={field.param_name}>
              <Text style={styles.fieldLabel}>{field.label.toUpperCase()}</Text>
              <TextInput
                style={styles.input}
                secureTextEntry
                placeholder={field.placeholder || "sk-••••••••••••••••••••"}
                placeholderTextColor={colors.textMuted}
                value={values[field.param_name] ?? ""}
                onChangeText={(text) => setValues((prev) => ({ ...prev, [field.param_name]: text }))}
                autoCapitalize="none"
                autoCorrect={false}
              />
            </View>
          ))}

          <View style={styles.actions}>
            <Pressable style={styles.cancelBtn} onPress={handleCancel}>
              <Text style={styles.cancelText}>Cancel</Text>
            </Pressable>
            <View style={styles.submitWrap}>
              <PrimaryButton label="Save Key" onPress={handleSubmit} />
            </View>
          </View>
        </View>
      </View>
    </Modal>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    backdrop: {
      flex: 1,
      backgroundColor: "rgba(0,0,0,0.65)",
      justifyContent: "center",
      alignItems: "center",
      padding: spacing.xxl,
    },
    // DESIGN.md §Dialogs: rounded-xl (12px), p-5, gap-4.
    modal: {
      width: 270,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.borderSoft,
      borderRadius: radius.lg,
      padding: spacing.xxxl,
      gap: spacing.xxl,
    },
    // 36×36 icon tile per DESIGN.md §Banners & callouts (success variant via theme token).
    iconWrap: {
      width: 36,
      height: 36,
      borderRadius: radius.md,
      backgroundColor: c.successTone.bg,
      borderWidth: 1,
      borderColor: c.successTone.border,
      alignItems: "center",
      justifyContent: "center",
      alignSelf: "center",
    },
    title: {
      ...typeScale.title,
      textAlign: "center",
      color: c.textPrimary,
    },
    desc: {
      ...typeScale.small,
      color: c.textSecondary,
      textAlign: "center",
    },
    descBold: {
      fontWeight: "700",
      color: c.textPrimary,
    },
    // Trust pill: DESIGN.md badge recipe via theme tone token.
    trustBadge: {
      flexDirection: "row",
      alignItems: "center",
      justifyContent: "center",
      gap: spacing.xs,
      paddingVertical: 2,
      paddingHorizontal: spacing.sm,
      borderRadius: radius.sm,
      backgroundColor: c.successTone.bg,
      borderWidth: 1,
      borderColor: c.successTone.border,
    },
    trustText: {
      ...typeScale.badge,
      color: c.successTone.text,
    },
    fieldLabel: {
      ...typeScale.overline,
      color: c.textMuted,
      letterSpacing: 0.6,
    },
    serviceRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
      paddingVertical: spacing.sm,
      paddingHorizontal: spacing.md,
      borderRadius: radius.md,
      backgroundColor: c.cardSoft,
      borderWidth: 1,
      borderColor: c.border,
    },
    serviceDot: {
      width: 20,
      height: 20,
      borderRadius: radius.sm,
      backgroundColor: c.infoSoft,
      alignItems: "center",
      justifyContent: "center",
    },
    serviceDotText: {
      ...typeScale.overline,
      color: c.info,
      letterSpacing: 0,
    },
    serviceText: {
      ...typeScale.label,
      color: c.textPrimary,
    },
    input: {
      paddingVertical: spacing.md,
      paddingHorizontal: spacing.lg,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: c.cardSoft,
      ...typeScale.mono,
      color: c.textMuted,
      letterSpacing: 2,
    },
    actions: {
      flexDirection: "row",
      gap: spacing.sm,
    },
    cancelBtn: {
      paddingHorizontal: spacing.xxl,
      minHeight: TOUCH_TARGET,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: "transparent",
      alignItems: "center",
      justifyContent: "center",
    },
    cancelText: {
      ...typeScale.label,
      color: c.textSecondary,
    },
    submitWrap: {
      flex: 1,
    },
  });
