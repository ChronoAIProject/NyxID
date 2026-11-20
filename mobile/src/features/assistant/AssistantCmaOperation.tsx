import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import { StyleSheet, View } from "react-native";
import { Card, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { cmaPresentation, isCmaOperationPending } from "./assistantCma";
import type { CmaOperation } from "./assistantTypes";

const phases: Record<string, string> = {
  get sandbox_creation() {
    return translateApp("Creating sandbox");
  },
  get sandbox_startup() {
    return translateApp("Starting sandbox");
  },
  get workspace_creation() {
    return translateApp("Preparing workspace");
  },
  get agent_creation() {
    return translateApp("Creating agent");
  },
  get agent_startup() {
    return translateApp("Starting agent");
  },
};
export function AssistantCmaOperation({
  operation,
}: {
  operation: CmaOperation;
}) {
  useAppLanguage();
  const { colors } = useTheme();
  const { label, tone } = cmaPresentation(operation);
  if (!["cma_response", "cma_agent", "cma_provisioning"].includes(operation.kind))
    return <Label muted>{label}</Label>;
  const pending = isCmaOperationPending(operation);
  const complete = tone === "success";
  const accepted =
    !!operation.operation_id ||
    (operation.kind === "cma_agent" && !!operation.status);
  const stages = [
    { label: translateApp("Submitted"), done: accepted },
    {
      label:
        operation.kind === "cma_provisioning" ? translateApp("Provisioning")
          : translateApp("CMA working"),
      done: complete,
    },
    {
      label: operation.kind === "cma_response" ? translateApp("Result ready")
          : translateApp("Agent ready"),
      done: complete,
    },
  ];
  return (
    <View accessibilityLabel={translateApp("CMA delegation")}>
      <Card>
        <View style={styles.row}>
          <Label style={styles.title}>{translateApp("Delegated to CMA")}</Label>
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
        <Label muted>{translateApp("ChronoAI Managed Agent")}</Label>
        {[
          [translateApp("Agent"), operation.agent_id],
          [
            translateApp("Operation"),
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
        <View accessibilityLabel={translateApp("CMA progress")}
          style={styles.stages}>
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
            {translateApp(
              "CMA’s final outcome could not be confirmed. Check the operation before submitting it again.",
            )}
          </Label>
        ) : null}
        {tone === "success" && operation.reason?.startsWith("result_") ? (
          <Label muted>
            {translateApp(
              "CMA finished, but its final result is unavailable here.",
            )}
          </Label>
        ) : null}
        {operation.state === "cancelled" ? (
          <Label muted>
            {translateApp("Tracking ended. CMA may still be running.")}
          </Label>
        ) : null}
        <Label muted>
          {(pending ? translateApp("{{name}} · Tracking progress", { name: operation.monitor_name}) : translateApp("{{name}} · Status recorded", { name: operation.monitor_name }))}
        </Label>
        {operation.checked_at &&
        Number.isFinite(Date.parse(operation.checked_at)) ? (
          <Label muted style={styles.small}>{translateApp("Checked {{time}}", { time: new Date(operation.checked_at).toLocaleTimeString(appLocale(), {
              hour: "2-digit",
              minute: "2-digit",
            }) })}
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
