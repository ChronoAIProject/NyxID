import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import {
  Check,
  ChevronDown,
  ChevronRight,
  ShieldCheck,
  X,
} from "lucide-react-native";
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
  turnFailed = false,
  metadata,
  decisions = [],
  confirmationActivityIds,
}: {
  activities: Activity[];
  onToggle: () => void;
  activityTotal?: number | null;
  turnLabel?: string;
  /** The turn itself failed, even if every action in it completed. */
  turnFailed?: boolean;
  metadata?: AssistantMessage["turn_metadata"];
  decisions?: AssistantAcknowledgement[];
  /** Calls that raised a confirmation card, including cards still pending. */
  confirmationActivityIds?: ReadonlySet<string>;
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
  // A call refused only to ask for a confirmation card paused; it did not fail.
  const gated = new Set(
    activities
      .filter(
        (activity) =>
          activity.status === "error" &&
          (confirmationActivityIds?.has(activity.id) ||
            decisions.some((row) => row.activity_id === activity.id)),
      )
      .map((activity) => activity.id),
  );
  const failed = activities.filter(
    (activity) => activity.status === "error" && !gated.has(activity.id),
  ).length;
  const status = activities.some((activity) => activity.status === "running")
    ? "running"
    : failed || turnFailed
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
    <View style={styles.container}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`Turn details: ${summary}`}
        accessibilityHint={open ? "Hide action details" : "Show action details"}
        accessibilityState={{ expanded: open }}
        hitSlop={{ top: 7, bottom: 7 }}
        onPress={() => {
          onToggle();
          setOpen((value) => !value);
        }}
        style={({ pressed }) => [
          styles.summary,
          { backgroundColor: colors.segmentTrack, opacity: pressed ? 0.65 : 1 },
        ]}
      >
        <View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        >
          <Chevron size={14} color={colors.textSecondary} />
        </View>
        <ActivityStatusIcon status={status} />
        <Label
          muted
          numberOfLines={1}
          maxFontSizeMultiplier={1.3}
          style={styles.count}
        >
          {summary}
          {total > activities.length
            ? ` · latest ${activities.length} of ${total} shown`
            : ""}
        </Label>
      </Pressable>
      {open ? (
        <View style={[styles.details, { borderColor: colors.borderSoft }]}>
          {activities.map((activity) => {
            const confirmation = gated.has(activity.id);
            const description = confirmation
              ? "Asked for your confirmation"
              : describeToolActivity(
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
                accessibilityLabel={`${description}, ${activity.label}, ${confirmation ? "Confirmation requested" : activity.status === "error" ? "Failed" : activity.status === "completed" ? "Succeeded" : "Running"}${elapsed ? `, ${elapsed}` : ""}`}
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
                <ActivityStatusIcon
                  status={confirmation ? "confirmation" : activity.status}
                />
                <View style={styles.activityText}>
                  <Label
                    muted
                    style={[
                      styles.description,
                      activity.status === "error" &&
                        !confirmation && { color: colors.danger },
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
                  {activity.status === "error" && !confirmation ? (
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

function ActivityStatusIcon({
  status,
}: {
  status: Activity["status"] | "confirmation";
}) {
  const { colors } = useTheme();
  return (
    <View
      style={styles.status}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {status === "running" ? (
        <ActivityIndicator size="small" color={colors.primary} />
      ) : status === "confirmation" ? (
        <ShieldCheck size={14} color={colors.textSecondary} />
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
  // A quiet one-line pill; details open beneath it on a thin left rule.
  container: { gap: 6 },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "flex-start",
    maxWidth: "100%",
    gap: 6,
    minHeight: TOUCH_TARGET - 14,
    paddingHorizontal: 10,
    paddingVertical: 3,
    borderRadius: radius.md + 6,
  },
  count: { fontSize: 12, lineHeight: 18, flexShrink: 1 },
  status: {
    width: 20,
    height: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  details: { borderLeftWidth: 2, paddingLeft: 12, paddingVertical: 4, gap: 8, marginLeft: 6 },
  activity: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  activityText: { flex: 1, gap: 2 },
  description: { fontSize: 12, lineHeight: 20 },
  name: { fontSize: 10, lineHeight: 16 },
});
