import { StyleSheet, View } from "react-native";
import { Card, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { cmaPresentation, isCmaOperationPending } from "./assistantCma";
import type { CmaOperation } from "./assistantTypes";

const phases: Record<string, string> = {
  sandbox_creation: "Creating sandbox",
  sandbox_startup: "Starting sandbox",
  workspace_creation: "Preparing workspace",
  agent_creation: "Creating agent",
  agent_startup: "Starting agent",
};
export function AssistantCmaOperation({
  operation,
}: {
  operation: CmaOperation;
}) {
  const { colors } = useTheme();
  const { label, tone } = cmaPresentation(operation);
  const pending = isCmaOperationPending(operation);
  const complete = tone === "success";
  const accepted =
    !!operation.operation_id ||
    (operation.kind === "cma_agent" && !!operation.status);
  const stages = [
    { label: "Submitted", done: accepted },
    {
      label:
        operation.kind === "cma_provisioning" ? "Provisioning" : "CMA working",
      done: complete,
    },
    {
      label: operation.kind === "cma_response" ? "Result ready" : "Agent ready",
      done: complete,
    },
  ];
  return (
    <View accessibilityLabel="CMA delegation">
      <Card>
        <View style={styles.row}>
          <Label style={styles.title}>Delegated to CMA</Label>
          <Label
            accessibilityLiveRegion="polite"
            style={{
              color:
                tone === "error"
                  ? colors.danger
                  : tone === "success"
                    ? colors.success
                    : tone === "warning"
                      ? colors.warning
                      : colors.textSecondary,
            }}
          >
            {label}
          </Label>
        </View>
        <Label muted>ChronoAI Managed Agent</Label>
        {[
          ["Agent", operation.agent_id],
          [
            "Operation",
            operation.operation_id !== operation.agent_id
              ? operation.operation_id
              : null,
          ],
          ["Idempotency-Key", operation.idempotency_key],
        ]
          .filter(([, value]) => value)
          .map(([name, value]) => (
            <Label key={name} muted mono selectable>
              {name} {value}
            </Label>
          ))}
        <View accessibilityLabel="CMA progress" style={styles.stages}>
          {stages.map((stage) => (
            <View
              key={stage.label}
              style={[
                styles.stage,
                {
                  borderColor: colors.border,
                  backgroundColor: colors.cardSoft,
                },
              ]}
            >
              <Label muted style={styles.small}>
                {stage.done ? "✓ " : "○ "}
                {stage.label}
              </Label>
            </View>
          ))}
        </View>
        {operation.phase && phases[operation.phase] && pending ? (
          <Label muted>{phases[operation.phase]}</Label>
        ) : null}
        {tone === "warning" && !pending ? (
          <Label muted>
            CMA’s final outcome could not be confirmed. Check the operation
            before submitting it again.
          </Label>
        ) : null}
        {tone === "success" && operation.reason?.startsWith("result_") ? (
          <Label muted>
            CMA finished, but its final result is unavailable here.
          </Label>
        ) : null}
        {operation.state === "cancelled" ? (
          <Label muted>Tracking ended. CMA may still be running.</Label>
        ) : null}
        <Label muted>
          {operation.monitor_name} ·{" "}
          {pending ? "Tracking progress" : "Status recorded"}
        </Label>
        {operation.checked_at &&
        Number.isFinite(Date.parse(operation.checked_at)) ? (
          <Label muted style={styles.small}>
            Checked{" "}
            {new Date(operation.checked_at).toLocaleTimeString([], {
              hour: "2-digit",
              minute: "2-digit",
            })}
          </Label>
        ) : null}
      </Card>
    </View>
  );
}
const styles = StyleSheet.create({
  row: { flexDirection: "row", flexWrap: "wrap", gap: 8, alignItems: "center" },
  title: { fontSize: 14, fontWeight: "600", flexGrow: 1 },
  stages: { flexDirection: "row", flexWrap: "wrap", gap: 6 },
  stage: { padding: 8, borderRadius: 8, borderWidth: 1 },
  small: { fontSize: 11 },
});
