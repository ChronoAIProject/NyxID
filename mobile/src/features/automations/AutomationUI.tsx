import { useReducedMotion } from "../../lib/accessibility/useReducedMotion";
import {
  useEffect,
  useState,
  type ReactNode,
  type PropsWithChildren,
} from "react";
import {
  ActivityIndicator,
  AppState,
  Modal,
  Pressable,
  ScrollView,
  StyleSheet,
  View,
} from "react-native";
import { useIsFocused } from "@react-navigation/native";
import { SafeAreaProvider, SafeAreaView } from "react-native-safe-area-context";
import { Check, ChevronDown, X } from "lucide-react-native";
import { z } from "zod";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";

export const AUTOMATIONS = ["workspace", "automations"] as const;
export const AUTOMATION_SETTINGS = [...AUTOMATIONS, "settings"] as const;
export function automationError(error: unknown): string {
  if (error instanceof z.ZodError)
    return error.issues[0]?.message ?? "Could not read automations.";
  return error instanceof Error
    ? error.message
    : "Could not load automations. Try again.";
}

/** Poll only while the page is visible and the app is active. */
export function useAutomationRuntime() {
  const focused = useIsFocused();
  const [foreground, setForeground] = useState(
    AppState.currentState === "active",
  );
  const { isConnected } = useNetworkStatus();
  useEffect(() => {
    const subscription = AppState.addEventListener("change", (value) =>
      setForeground(value === "active"),
    );
    return () => subscription.remove();
  }, []);
  return { active: focused && foreground, online: isConnected };
}

export type Choice = {
  leading?: ReactNode;
  value: string;
  label: string;
  description?: string;
};
export function ChoiceField({
  label,
  value,
  options,
  onChange,
  disabled = false,
}: {
  label: string;
  value: string;
  options: readonly Choice[];
  onChange: (value: string) => void;
  disabled?: boolean;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const current = options.find((option) => option.value === value);
  return (
    <View style={automationStyles.field}>
      <Label>{label}</Label>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${label}: ${current?.label ?? "Choose"}`}
        accessibilityState={{ disabled, expanded: open }}
        disabled={disabled}
        onPress={() => setOpen(true)}
        style={[
          automationStyles.choice,
          {
            backgroundColor: colors.cardSoft,
            borderColor: colors.border,
            opacity: disabled ? 0.5 : 1,
          },
        ]}
      >
        {current?.leading}
        <Label style={automationStyles.flex}>
          {current?.label ?? "Choose"}
        </Label>
        <ChevronDown size={17} color={colors.textSecondary} />
      </Pressable>
      <AutomationSheet
        title={label}
        visible={open}
        onClose={() => setOpen(false)}
      >
        {options.map((option) => (
          <Pressable
            key={option.value}
            accessibilityRole="radio"
            accessibilityState={{ checked: option.value === value }}
            disabled={disabled}
            onPress={() => {
              if (!disabled) onChange(option.value);
              setOpen(false);
            }}
            style={[automationStyles.option, { borderColor: colors.border }]}
          >
            {option.leading}
            <View style={automationStyles.flex}>
              <Label>{option.label}</Label>
              {option.description ? (
                <Label muted style={automationStyles.small}>
                  {option.description}
                </Label>
              ) : null}
            </View>
            {option.value === value && (
              <Check color={colors.primary} size={18} />
            )}
          </Pressable>
        ))}
        {!options.length && <Label muted>No options are available.</Label>}
      </AutomationSheet>
    </View>
  );
}

export function AutomationSheet({
  title,
  visible,
  onClose,
  children,
}: PropsWithChildren<{
  title: string;
  visible: boolean;
  onClose: () => void;
}>) {
  const { colors } = useTheme();
  const reducedMotion = useReducedMotion();
  return (
    <Modal
      visible={visible}
      animationType={reducedMotion ? "none" : "slide"}
      presentationStyle="pageSheet"
      onRequestClose={onClose}
    >
      <SafeAreaProvider>
        <SafeAreaView
          style={[automationStyles.fill, { backgroundColor: colors.bg }]}
        >
          <View style={automationStyles.sheetHeader}>
            <Label accessibilityRole="header" style={automationStyles.section}>
              {title}
            </Label>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={`Close ${title}`}
              onPress={onClose}
              style={automationStyles.iconButton}
            >
              <X size={22} color={colors.textPrimary} />
            </Pressable>
          </View>
          <ScrollView
            keyboardShouldPersistTaps="handled"
            contentContainerStyle={automationStyles.sheetBody}
          >
            {children}
          </ScrollView>
        </SafeAreaView>
      </SafeAreaProvider>
    </Modal>
  );
}

export function AutomationStatus({ value }: { value: string }) {
  const { colors } = useTheme();
  const palette =
    value === "active" || value === "completed"
      ? colors.successTone
      : value === "failed"
        ? colors.dangerTone
        : colors.infoTone;
  const text = value === "disabled" ? "Paused" : value.replaceAll("_", " ");
  return (
    <View
      style={[
        automationStyles.badge,
        { backgroundColor: palette.bg, borderColor: palette.border },
      ]}
    >
      <Label style={[automationStyles.badgeText, { color: palette.text }]}>
        {text.charAt(0).toUpperCase() + text.slice(1)}
      </Label>
    </View>
  );
}

export function AutomationLoading() {
  const { colors } = useTheme();
  return (
    <ActivityIndicator
      accessibilityLabel="Loading automations"
      color={colors.primary}
      style={{ padding: 24 }}
    />
  );
}

export const automationStyles = StyleSheet.create({
  fill: { flex: 1 },
  flex: { flex: 1, minWidth: 0 },
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  spread: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 12,
  },
  field: { gap: 8 },
  section: { fontFamily: fonts.displaySemi, fontSize: 18, lineHeight: 25 },
  title: { fontFamily: fonts.displaySemi, fontSize: 16, lineHeight: 23 },
  small: { fontSize: 12, lineHeight: 19 },
  choice: {
    minHeight: 48,
    borderWidth: 1,
    borderRadius: 8,
    paddingHorizontal: 14,
    paddingVertical: 10,
    flexDirection: "row",
    alignItems: "center",
    gap: 10,
  },
  iconButton: {
    width: 44,
    height: 44,
    alignItems: "center",
    justifyContent: "center",
  },
  option: {
    minHeight: 56,
    paddingVertical: 14,
    borderBottomWidth: StyleSheet.hairlineWidth,
    flexDirection: "row",
    alignItems: "center",
    gap: 12,
  },
  sheetHeader: {
    paddingLeft: 24,
    paddingRight: 10,
    paddingVertical: 8,
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
  },
  sheetBody: { padding: 24, gap: 16 },
  badge: {
    borderWidth: 1,
    borderRadius: 6,
    paddingHorizontal: 8,
    paddingVertical: 3,
    alignSelf: "flex-start",
  },
  badgeText: { fontSize: 11, lineHeight: 17 },
});
