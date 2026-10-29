import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { Check, ChevronDown, ChevronRight, X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius, TOUCH_TARGET } from "../../theme/designTokens";
import type {
  AssistantAcknowledgement,
  AssistantMessage,
} from "./assistantTypes";
import {
  countDecisions,
  decisionSummary,
  decisionTitle,
  groupDecisions,
} from "./assistantDecisions";
import { describeToolActivity } from "./assistantActivityText";
import {
  deliveryText,
  elapsedLabel,
  formatCapacityWait,
  toolFailureCopy,
} from "./assistantTurnPresentation";

type Activity = Omit<AssistantMessage["activities"][number], "machine">;

export function AssistantActivityDisclosure({
  activities,
  onToggle,
  activityTotal,
  turnLabel,
  metadata,
  decisions = [],
}: {
  activities: Activity[];
  onToggle: () => void;
  activityTotal?: number | null;
  turnLabel?: string;
  metadata?: AssistantMessage["turn_metadata"];
  decisions?: AssistantAcknowledgement[];
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  const [now, setNow] = useState(Date.now);
  const ticking = activities.some((activity) => activity.status === "running");
  useEffect(() => {
    if (!ticking) return;
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, [ticking]);
  const delivery = metadata?.channel_delivery;
  const waited =
    (metadata?.capacity_wait_ms ?? 0) > 0 || (metadata?.attempts ?? 0) > 1;
  if (!activities.length && !waited && !delivery && !decisions.length)
    return null;
  const failed = activities.filter(
    (activity) => activity.status === "error",
  ).length;
  const status = activities.some((activity) => activity.status === "running")
    ? "running"
    : failed
      ? "error"
      : "completed";
  const total = Math.max(activities.length, activityTotal ?? 0);
  const count = `${total} ${total === 1 ? "action" : "actions"}`;
  const label = turnLabel ?? (status === "running" ? "Working…" : "Completed");
  const summary = [
    label,
    total ? count : "",
    ...decisionSummary(countDecisions(decisions)),
    waited && metadata?.capacity_wait_ms
      ? `waited ${formatCapacityWait(metadata.capacity_wait_ms)}`
      : "",
    failed ? `${failed} failed` : "",
    delivery?.status === "failed" ? "delivery failed" : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const Chevron = open ? ChevronDown : ChevronRight;

  return (
    <View
      style={[
        styles.container,
        { backgroundColor: colors.cardSoft, borderColor: colors.border },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Turn details: ${summary}`}
        accessibilityHint={open ? "Hide action details" : "Show action details"}
        accessibilityState={{ expanded: open }}
        onPress={() => {
          onToggle();
          setOpen((value) => !value);
        }}
        style={({ pressed }) => [
          styles.summary,
          { opacity: pressed ? 0.65 : 1 },
        ]}
      >
        <View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        >
          <Chevron size={14} color={colors.textSecondary} />
        </View>
        <ActivityStatusIcon status={status} />
        <Label muted style={styles.count}>
          {summary}
          {total > activities.length
            ? ` · latest ${activities.length} of ${total} shown`
            : ""}
        </Label>
      </Pressable>
      {open ? (
        <View style={[styles.details, { borderColor: colors.border }]}>
          {activities.map((activity) => {
            const description = describeToolActivity(
              activity.label,
              activity.status === "completed" ? "done" : activity.status,
            );
            const elapsed = elapsedLabel(
              activity.started_at,
              activity.status === "running"
                ? undefined
                : (activity.ended_at ?? activity.started_at),
              now,
            );
            return (
              <View
                key={activity.id}
                accessible
                accessibilityRole="text"
                accessibilityLabel={`${description}, ${activity.label}, ${activity.status === "error" ? "Failed" : activity.status === "completed" ? "Succeeded" : "Running"}${elapsed ? `, ${elapsed}` : ""}`}
                style={styles.activity}
              >
                <ServiceIcon
                  slug={
                    activity.label.includes("__") &&
                    !activity.label.startsWith("nyx__")
                      ? activity.label.split("__")[0]
                      : undefined
                  }
                  size="xs"
                />
                <ActivityStatusIcon status={activity.status} />
                <View style={styles.activityText}>
                  <Label
                    muted
                    style={[
                      styles.description,
                      activity.status === "error" && { color: colors.danger },
                    ]}
                  >
                    {description}
                    {elapsed ? ` · ${elapsed}` : ""}
                  </Label>
                  <Label muted mono style={styles.name}>
                    {activity.label}
                  </Label>
                  {decisions
                    .filter((row) => row.activity_id === activity.id)
                    .map((row) => (
                      <Label
                        key={row.id}
                        muted
                        accessibilityLabel={decisionTitle(row)}
                        style={styles.description}
                      >
                        {decisionTitle(row)}
                      </Label>
                    ))}
                  {activity.status === "error" ? (
                    <Label
                      style={[styles.description, { color: colors.danger }]}
                    >
                      {toolFailureCopy(activity.failure)} The assistant can
                      continue after a failed step.
                    </Label>
                  ) : null}
                </View>
              </View>
            );
          })}
          {groupDecisions(
            decisions.filter(
              (row) =>
                !activities.some((activity) => activity.id === row.activity_id),
            ),
          ).map(({ row, count }) => (
            <Label
              key={row.id}
              muted
              accessibilityLabel={decisionTitle(row)}
              style={styles.description}
            >
              {decisionTitle(row)}
              {count > 1 ? ` ×${count}` : ""}
            </Label>
          ))}
          {waited ? (
            <Label muted style={styles.description}>
              {metadata?.attempts != null
                ? `${metadata.attempts} attempts`
                : ""}
              {metadata?.capacity_wait_ms
                ? ` · waited ${formatCapacityWait(metadata.capacity_wait_ms)} for capacity`
                : ""}
            </Label>
          ) : null}
          {delivery ? <DeliveryNotice delivery={delivery} /> : null}
        </View>
      ) : null}
    </View>
  );
}

function ActivityStatusIcon({ status }: { status: Activity["status"] }) {
  const { colors } = useTheme();
  return (
    <View
      style={styles.status}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {status === "running" ? (
        <ActivityIndicator size="small" color={colors.primary} />
      ) : status === "error" ? (
        <X size={14} color={colors.danger} />
      ) : (
        <Check size={14} color={colors.success} />
      )}
    </View>
  );
}

function DeliveryNotice({
  delivery,
}: {
  delivery: NonNullable<
    NonNullable<AssistantMessage["turn_metadata"]>["channel_delivery"]
  >;
}) {
  return (
    <Label muted style={styles.description}>
      {deliveryText(delivery)}
    </Label>
  );
}

const styles = StyleSheet.create({
  container: { borderWidth: 1, borderRadius: radius.md, overflow: "hidden" },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: TOUCH_TARGET,
    paddingHorizontal: 10,
    paddingVertical: 6,
  },
  count: { fontSize: 12, lineHeight: 18, flexShrink: 1 },
  status: {
    width: 20,
    height: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  details: { borderTopWidth: 1, padding: 10, gap: 8 },
  activity: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  activityText: { flex: 1, gap: 2 },
  description: { fontSize: 12, lineHeight: 20 },
  name: { fontSize: 10, lineHeight: 16 },
});
