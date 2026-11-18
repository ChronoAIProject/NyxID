import { ServiceIcon } from "../../components/ServiceIcon";
import type { ReactNode } from "react";
import { StyleSheet, View } from "react-native";
import {
  Card,
  Label,
  ActionButton,
} from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import type { AssistantAcknowledgement } from "./assistantTypes";

export function acknowledgementCardTitle(
  row: AssistantAcknowledgement,
): string {
  if (row.decider === "orchestrator") {
    if (row.kind === "service")
      return `Allow this agent to use ${row.service_name ?? row.service_slug ?? "this service"}?`;
    if (row.kind === "account")
      return "Allow this agent to read your NyxID account?";
    return `Confirm: ${row.summary}`;
  }
  if (row.kind === "service")
    return `Allow this chat to use ${row.service_name ?? "this service"}?`;
  if (row.kind === "account")
    return "Allow this chat to manage your NyxID account (keys, channel bots, services, nodes, approval settings)?";
  return `Confirm: ${row.summary}`;
}
export function acknowledgementHelp(row: AssistantAcknowledgement): string {
  const routed = row.decider === "orchestrator";
  return (
    (row.kind === "skills"
      ? routed
        ? "Allowing this proposal asks NyxBot to resolve the exact version. Adding or re-pinning the skill follows NyxBot's permission mode."
        : "Allowing records your decision only; it attaches nothing by itself. Add the skill in the agent's Skills editor."
      : row.kind === "operations"
        ? "This changes the specialist's operations on every thread. Widening follows NyxBot's permission mode."
        : routed
          ? "NyxBot decides this specialist's request against what you asked for. You can decide it here instead; the agent resumes on its own."
          : row.kind === "action"
            ? "This confirmation applies once, to this action only."
            : "This permission applies to this chat only.") +
    (routed
      ? ""
      : " Allowing lets the assistant continue as soon as it finishes its reply.")
  );
}
function confirmationReason(row: AssistantAcknowledgement): string {
  if (row.confirmation?.reason === "webhook")
    return "This automation's confirmation policy asks.";
  if (row.confirmation?.reason === "policy")
    return "This machine's or saved login's confirmation setting asks.";
  const labels = {
    autopilot: "Full access",
    nyxmode: "Auto",
    ask_every_time: "Ask every time",
  };
  return row.confirmation?.access_mode
    ? `Your access mode is ${labels[row.confirmation.access_mode]}.`
    : "Your access mode asks.";
}
export function AssistantAcknowledgementCard({
  row,
  selected,
  disabled,
  deciding,
  onDecision,
  authoredReview,
}: {
  row: AssistantAcknowledgement;
  selected: boolean;
  disabled: boolean;
  deciding: boolean;
  onDecision: (choice: "allow" | "deny") => void;
  authoredReview?: ReactNode;
}) {
  const { colors } = useTheme();
  if (row.status !== "pending") return null;
  const expired = Date.parse(row.expires_at) <= Date.now();
  return (
    <View accessibilityLabel={acknowledgementCardTitle(row)}>
      <Card style={selected ? { borderColor: colors.primary } : undefined}>
        {row.decider === "orchestrator" ? (
          <Label muted>Requested from NyxBot</Label>
        ) : null}
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <ServiceIcon slug={row.service_slug} size="sm" />
          <Label style={[styles.title, { flex: 1 }]}>
            {acknowledgementCardTitle(row)}
          </Label>
        </View>
        {row.confirmation ? (
          <Label muted>
            {row.confirmation.risk.charAt(0).toUpperCase() +
              row.confirmation.risk.slice(1)}{" "}
            · {confirmationReason(row)}
          </Label>
        ) : null}
        {authoredReview ?? (
          <>
            <Label muted>{acknowledgementHelp(row)}</Label>
            {row.tool_name ? (
              <Label muted mono>
                {row.tool_name}
              </Label>
            ) : null}
            <Label muted>
              Expires {new Date(row.expires_at).toLocaleString()}
            </Label>
            {expired ? (
              <Label muted>
                This request expired. Refresh for its latest state.
              </Label>
            ) : null}
            <View style={styles.row}>
              <ActionButton
                label="Deny"
                variant="secondary"
                disabled={disabled || expired}
                onPress={() => onDecision("deny")}
              />
              <ActionButton
                label="Allow"
                loading={deciding}
                disabled={disabled || expired}
                onPress={() => onDecision("allow")}
              />
            </View>
          </>
        )}
      </Card>
    </View>
  );
}
const styles = StyleSheet.create({
  title: { fontSize: 13, fontWeight: "600" },
  row: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: 8,
    justifyContent: "flex-end",
  },
});
