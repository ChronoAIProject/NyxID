import type { PropsWithChildren, ReactNode } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { ChevronRight } from "lucide-react-native";
import { SectionBadge } from "../../components/SectionBadge";
import { StatusBadge } from "../../components/StatusBadge";
import { Card, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts, TOUCH_TARGET, typeScale } from "../../theme/designTokens";
import type { MachineNode } from "./machineContracts";

export const machineTabs = [{ label: "Machines" }, { label: "Saved logins" }];

export { WorkspaceDrawerHeader as MachineDrawerHeader } from "../../components/workspace/WorkspaceDrawerHeader";

export function MachineActionRow({
  label,
  onPress,
  disabled = false,
  chevron = false,
}: {
  label: string;
  onPress: () => void;
  disabled?: boolean;
  chevron?: boolean;
}) {
  const { colors } = useTheme();
  return (
    <Pressable
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ disabled }}
      disabled={disabled}
      onPress={onPress}
      style={({ pressed }) => [
        machineStyles.actionRow,
        { opacity: pressed ? 0.7 : 1 },
      ]}
    >
      <Label
        style={[
          machineStyles.actionLabel,
          { color: disabled ? colors.textTertiary : colors.textSecondary },
          !chevron && machineStyles.textAction,
        ]}
      >
        {label}
      </Label>
      {chevron && <ChevronRight size={16} color={colors.textMuted} />}
    </Pressable>
  );
}

export function machineIsolationLabel(node: MachineNode) {
  if (!node.machine?.shell) return "Commands off";
  return node.machine.commands_isolated === true
    ? "Isolated"
    : node.machine.commands_isolated === false
      ? "Not isolated"
      : "Isolation unknown";
}

export function MachineBadges({ node }: { node: MachineNode }) {
  const machine = node.machine!;
  return (
    <>
      <StatusBadge
        variant={node.is_connected ? "decisionApproved" : "decisionExpired"}
        label={node.is_connected ? "Online" : "Offline"}
      />
      {machine.shell &&
        (machine.commands_isolated === true ? (
          <SectionBadge label="Isolated" tone="success" />
        ) : (
          <SectionBadge label={machineIsolationLabel(node)} tone="warning" />
        ))}
    </>
  );
}

export function MachineMetadata({ node }: { node: MachineNode }) {
  return (
    <Label muted numberOfLines={1} style={machineStyles.metadata}>
      {node.owner.display_name} · {node.machine?.os}/{node.machine?.arch} · v
      {node.metadata?.agent_version ?? "unknown"}
    </Label>
  );
}

/** Keep navigation and secondary actions as separate accessibility targets. */
export function MachineRowCard({
  children,
  label,
  summary,
  onPress,
  actions,
}: PropsWithChildren<{
  label: string;
  summary?: string;
  onPress: () => void;
  actions?: ReactNode;
}>) {
  const { colors } = useTheme();
  return (
    <Card style={machineStyles.rowCard}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={summary ? `${label}. ${summary}` : label}
        onPress={onPress}
        style={({ pressed }) => [
          machineStyles.cardTarget,
          { opacity: pressed ? 0.7 : 1 },
        ]}
      >
        <View style={machineStyles.rowContent}>{children}</View>
        <ChevronRight size={16} color={colors.textMuted} />
      </Pressable>
      {actions && <View style={machineStyles.cardActions}>{actions}</View>}
    </Card>
  );
}

export function MachineCapability({
  label,
  available,
}: {
  label: string;
  available: boolean;
}) {
  return (
    <View style={machineStyles.capability}>
      <Label style={machineStyles.metadata}>{label}</Label>
      <StatusBadge
        label={available ? "Available" : "Off"}
        variant={available ? "decisionApproved" : "decisionExpired"}
      />
    </View>
  );
}

export const machineStyles = StyleSheet.create({
  actionRow: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: TOUCH_TARGET,
  },
  actionLabel: { ...typeScale.body, flexShrink: 1 },
  textAction: { textDecorationLine: "underline" },
  row: { flexDirection: "row", alignItems: "center", gap: 6 },
  badges: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: 6,
  },
  actions: { flexDirection: "row", flexWrap: "wrap", gap: 8 },
  name: { ...typeScale.bodyStrong, flex: 1, minWidth: 0 },
  heading: { ...typeScale.title },
  metadata: { ...typeScale.caption },
  rowCard: { padding: 0, gap: 0, overflow: "hidden" },
  cardTarget: {
    minHeight: TOUCH_TARGET,
    padding: 14,
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
  },
  rowContent: { flex: 1, minWidth: 0, gap: 8 },
  cardActions: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    paddingHorizontal: 14,
    paddingBottom: 12,
  },
  capability: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    gap: 8,
    flexGrow: 1,
    flexBasis: "45%",
    minWidth: 130,
  },
  grid: { flexDirection: "row", flexWrap: "wrap", gap: 12 },
});
