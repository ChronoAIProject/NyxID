import { StyleSheet, Switch } from "react-native";
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
