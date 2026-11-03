import {
  ActionSheetIOS,
  Alert,
  Platform,
  StyleSheet,
  Switch,
} from "react-native";
import { ChevronsUpDown } from "lucide-react-native";
import { SettingsRow } from "../../components/settings/SettingsList";
import { useTheme } from "../../theme/ThemeContext";
import { spacing } from "../../theme/designTokens";
import type { MachineNode } from "./machineContracts";

export const machineTabs = [{ label: "Machines" }, { label: "Saved logins" }];

/** Workspace pages keep their frame; content follows the Settings grouped-list rhythm. */
export const machinePageContent = StyleSheet.create({
  content: {
    paddingTop: spacing.sm,
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.huge,
    gap: spacing.huge,
  },
}).content;

export function machineIsolationLabel(node: MachineNode) {
  if (!node.machine?.shell) return "Commands off";
  return node.machine.commands_isolated === true
    ? "Isolated"
    : node.machine.commands_isolated === false
      ? "Not isolated"
      : "Isolation unknown";
}

export function machineConnectivity(node: MachineNode) {
  return node.is_connected ? "Online" : "Offline";
}

export function formatMachineDate(value: string | null | undefined) {
  if (!value) return "Never";
  const date = new Date(value);
  return Number.isFinite(date.getTime()) ? date.toLocaleString() : value;
}

/**
 * A short choice opens a native menu, as iOS pop-up buttons do, instead of
 * spreading its options over several rows.
 */
export function chooseFromMenu<T extends string>(
  title: string,
  options: readonly { value: T; label: string }[],
  onPick: (value: T) => void,
) {
  const labels = options.map((option) => option.label);
  if (Platform.OS === "ios") {
    ActionSheetIOS.showActionSheetWithOptions(
      {
        title,
        options: [...labels, "Cancel"],
        cancelButtonIndex: labels.length,
      },
      (index) => {
        const option = options[index];
        if (option) onPick(option.value);
      },
    );
    return;
  }
  Alert.alert(title, undefined, [
    ...options.map((option) => ({
      text: option.label,
      onPress: () => onPick(option.value),
    })),
    { text: "Cancel", style: "cancel" as const },
  ]);
}

/** One row holding its current value; tapping opens the native menu. */
export function MachineMenuRow({
  label,
  value,
  disabled,
  onPress,
}: {
  label: string;
  value: string;
  disabled?: boolean;
  onPress: () => void;
}) {
  const { colors } = useTheme();
  return (
    <SettingsRow
      label={label}
      value={value}
      disabled={disabled}
      accessibilityHint={`Opens a menu to choose ${label.toLowerCase()}`}
      accessory={<ChevronsUpDown size={16} color={colors.textMuted} />}
      onPress={onPress}
    />
  );
}

/** An inline switch row; switches never open a page. */
export function MachineSwitch({
  label,
  detail,
  accessibilityLabel,
  value,
  disabled,
  onChange,
}: {
  label: string;
  detail?: string;
  /** Spoken label when the visible label is shortened. */
  accessibilityLabel?: string;
  value: boolean;
  disabled?: boolean;
  onChange: (value: boolean) => void;
}) {
  const { colors } = useTheme();
  return (
    <SettingsRow
      label={label}
      detail={detail}
      disabled={disabled}
      accessory={
        <Switch
          accessibilityLabel={accessibilityLabel ?? label}
          value={value}
          disabled={disabled}
          onValueChange={onChange}
          trackColor={{ false: colors.borderSoft, true: colors.success }}
        />
      }
    />
  );
}
