import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
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
      return translateApp("Allow this agent to use {{service}}?", { service: row.service_name ?? row.service_slug ?? translateApp("this service") });
    if (row.kind === "account")
      return translateApp("Allow this agent to read your NyxID account?");
    return translateApp("Confirm: {{summary}}", { summary: row.summary});
  }
  if (row.kind === "service")
    return translateApp("Allow this chat to use {{service}}?", { service: row.service_name ?? translateApp("this service") });
  if (row.kind === "account")
    return translateApp(
      "Allow this chat to manage your NyxID account (keys, channel bots, services, nodes, approval settings)?",
    );
  return translateApp("Confirm: {{summary}}", { summary: row.summary});
}
export function acknowledgementHelp(row: AssistantAcknowledgement): string {
  const routed = row.decider === "orchestrator";
  return (
    (row.kind === "skills"
      ? (routed
        ? (routed ? translateApp("Allowing this proposal asks NyxBot to resolve the exact version. Adding or re-pinning the skill follows NyxBot's permission mode.") : translateApp("Allowing this proposal asks NyxBot to resolve the exact version. Adding or re-pinning the skill follows NyxBot's permission mode. Allowing lets the assistant continue as soon as it finishes its reply.")) : (routed ? translateApp("Allowing records your decision only; it attaches nothing by itself. Add the skill in the agent's Skills editor.") : translateApp("Allowing records your decision only; it attaches nothing by itself. Add the skill in the agent's Skills editor. Allowing lets the assistant continue as soon as it finishes its reply."))) : (row.kind === "operations"
        ? (routed
          ? translateApp("This changes the specialist's operations on every thread. Widening follows NyxBot's permission mode.") : translateApp("This changes the specialist's operations on every thread. Widening follows NyxBot's permission mode. Allowing lets the assistant continue as soon as it finishes its reply.")) : (routed ? (routed ? translateApp("NyxBot decides this specialist's request against what you asked for. You can decide it here instead; the agent resumes on its own.") : translateApp("NyxBot decides this specialist's request against what you asked for. You can decide it here instead; the agent resumes on its own. Allowing lets the assistant continue as soon as it finishes its reply.")) : (row.kind === "action"
            ? (routed
      ? translateApp("This confirmation applies once, to this action only.") : translateApp("This confirmation applies once, to this action only. Allowing lets the assistant continue as soon as it finishes its reply.")) : (routed ? translateApp("This permission applies to this chat only.") : translateApp("This permission applies to this chat only. Allowing lets the assistant continue as soon as it finishes its reply."))))))
  );
}
function confirmationReason(row: AssistantAcknowledgement): string {
  if (row.confirmation?.reason === "webhook")
    return translateApp("This automation's confirmation policy asks.");
  if (row.confirmation?.reason === "policy")
    return translateApp(
      "This machine's or saved login's confirmation setting asks.",
    );
  const labels = {
    autopilot: translateApp("Full access"),
    nyxmode: translateApp("Auto"),
    ask_every_time: translateApp("Ask every time"),
  };
  return row.confirmation?.access_mode
    ? translateApp("Your access mode is {{label}}.", {
        label: labels[row.confirmation.access_mode],
      })
    : translateApp("Your access mode asks.");
}
function confirmationRisk(
  risk: NonNullable<AssistantAcknowledgement["confirmation"]>["risk"],
) {
  return {
    read: translateApp("Read"),
    change: translateApp("Change"),
    sensitive: translateApp("Sensitive"),
    destructive: translateApp("Destructive"),
  }[risk];
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
  useAppLanguage();
  const { colors } = useTheme();
  if (row.status !== "pending") return null;
  const expired = Date.parse(row.expires_at) <= Date.now();
  return (
    <View accessibilityLabel={acknowledgementCardTitle(row)}>
      <Card style={selected ? { borderColor: colors.primary } : undefined}>
        {row.decider === "orchestrator" ? (
          <Label muted>{translateApp("Requested from NyxBot")}</Label>
        ) : null}
        <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
          <ServiceIcon slug={row.service_slug} size="sm" />
          <Label style={[styles.title, { flex: 1 }]}>
            {acknowledgementCardTitle(row)}
          </Label>
        </View>
        {row.confirmation ? (
          <Label muted>
            {confirmationRisk(row.confirmation.risk)} ·{" "}
            {confirmationReason(row)}
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
              {translateApp("Expires {{date}}", {
                date: new Date(row.expires_at).toLocaleString(appLocale()),
              })}
            </Label>
            {expired ? (
              <Label muted>
                {translateApp(
                  "This request expired. Refresh for its latest state.",
                )}
              </Label>
            ) : null}
            <View style={styles.row}>
              <ActionButton
                label={translateApp("Deny")}
                variant="secondary"
                disabled={disabled || expired}
                onPress={() => onDecision("deny")}
              />
              <ActionButton
                label={translateApp("Allow")}
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
