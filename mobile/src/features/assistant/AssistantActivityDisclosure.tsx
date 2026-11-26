import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { activityPresentationStatus } from "./assistantHistoryCompat";
import { activityDecisionPresentation } from "./assistantActivityDecision";
import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import {
  Check,
  CircleAlert,
  Minus,
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
  useAppLanguage();
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
          !activity.decision && activity.status === "error" &&
          (confirmationActivityIds?.has(activity.id) ||
            decisions.some((row) => row.activity_id === activity.id)),
      )
      .map((activity) => activity.id),
  );
  const failed = activities.filter(
    (activity) => activityPresentationStatus(activityDecisionPresentation(activity.decision)?.status ?? activity.status, activity.failure?.kind) === "error" && !gated.has(activity.id),
  ).length;
  const status = turnLabel === translateApp("Status unavailable")
      ? "unknown" : activities.some((activity) => activity.status === "running")
    ? "running"
    : failed || turnFailed
      ? "error"
      : !turnLabel && activities.some((activity) => activity.status === "unknown") ? "unknown" : "completed";
  const total = Math.max(activities.length, activityTotal ?? 0);
  const count = translateApp(
    total === 1 ? "{{count}} action" : "{{count}} actions",
    { count: total },
  );
  const label = turnLabel ?? (status === "running" ? translateApp("Working…")
      : status === "unknown" ? translateApp("Status unavailable")
        : translateApp("Completed"));
  const summary = [
    label,
    total ? count : "",
    ...decisionSummary(countDecisions(decisions, activities.flatMap((activity) => activity.decision ? [activity.decision] : []))),
    waited && metadata?.capacity_wait_ms
      ? translateApp("waited {{duration}}", {
          duration: formatCapacityWait(metadata.capacity_wait_ms),
        })
      : "",
    failed ? translateApp("{{failed}} failed", {failed: failed }) : "",
    delivery?.status === "failed" ? translateApp("delivery failed") : "",
  ]
    .filter(Boolean)
    .join(" · ");
  const Chevron = open ? ChevronDown : ChevronRight;

  return (
    <View style={styles.container}>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={translateApp("Turn details: {{summary}}", {summary: summary,
        })}
        accessibilityHint={open ? translateApp("Hide action details")
            : translateApp("Show action details")
        }
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
        <ActivityStatusIcon status={status} />
        <Label
          muted
          numberOfLines={1}
          maxFontSizeMultiplier={1.3}
          style={styles.count}
        >
          {(total > activities.length
            ? translateApp("{{summary}} · latest {{count}} of {{total}} shown", { summary: summary, count: activities.length, total: total }) : summary)}
        </Label>
        <View
          accessibilityElementsHidden
          importantForAccessibility="no-hide-descendants"
        >
          <Chevron size={16} color={colors.textSecondary} />
        </View>
      </Pressable>
      {open ? (
        <View style={styles.details}>
          {activities.map((activity) => {
            const decision = activityDecisionPresentation(activity.decision);
            const confirmation = Boolean(decision && decision.status === "waiting") || gated.has(activity.id);
            const status = activityPresentationStatus(decision ? decision.status === "done" ? "completed" : decision.status === "error" ? "error" : "running" : activity.status, activity.failure?.kind);
            const description = status === "skipped" ? toolFailureCopy(activity.failure, activity.label) : status === "unknown" ? translateApp(
                      "Activity details unavailable in this app version.",
                    )
                  : (decision?.label ?? (confirmation
              ? translateApp("Asked for your confirmation")
                      : describeToolActivity(
                  activity.label,
                  activity.status === "completed" ? "done" : activity.status,
                        )));
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
                accessibilityLabel={(status === "skipped" ? (elapsed ? translateApp("{{description}}, {{label}}, Skipped, {{elapsed}}", { description: description, label: activity.label, elapsed: elapsed }) : translateApp("{{description}}, {{label}}, Skipped", { description: description, label: activity.label })) : (status === "unknown" ? (elapsed ? translateApp("{{description}}, {{label}}, Status unavailable, {{elapsed}}", { description: description, label: activity.label, elapsed: elapsed }) : translateApp("{{description}}, {{label}}, Status unavailable", { description: description, label: activity.label })) : (elapsed ? translateApp("{{description}}, {{label}}, {{status}}, {{elapsed}}", { description: description, label: activity.label, status: decision?.label ?? (confirmation ? translateApp("Confirmation requested") : activity.status === "error" ? translateApp("Failed") : activity.status === "completed" ? translateApp("Succeeded") : translateApp("Running")), elapsed: elapsed}) : translateApp("{{description}}, {{label}}, {{status}}", { description: description, label: activity.label, status: decision?.label ?? (confirmation ? translateApp("Confirmation requested") : activity.status === "error" ? translateApp("Failed") : activity.status === "completed" ? translateApp("Succeeded") : translateApp("Running")) }))))}
                style={styles.activity}
              >
                <ActivityStatusIcon
                  status={confirmation ? "confirmation" : status}
                />
                <View style={styles.activityText}>
                  <Label
                    muted
                    style={[
                      styles.description,
                      status === "error" &&
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
                  {decision && status !== "skipped" && status !== "unknown" ? (
                    <Label muted style={styles.description}>{decision.detail}</Label>
                  ) : status === "error" && !confirmation ? (
                    <Label
                      style={[styles.description, { color: colors.danger }]}
                    >
                      {translateApp("{{label}} The assistant can continue after a failed step.", { label: toolFailureCopy(activity.failure, activity.label)})}</Label>
                  ) : null}
                </View>
                {/* The connected service, trailing so every row's text aligns;
                    NyxID's own tools carry no service mark. */}
                {serviceSlug(activity.label) ? (
                  <ServiceIcon slug={serviceSlug(activity.label)} size="xs" />
                ) : null}
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
              {(metadata?.attempts != null
                ? (metadata?.capacity_wait_ms
                ? translateApp("{{count}} attempts · waited {{waitDuration}} for capacity", { count: metadata.attempts, waitDuration: formatCapacityWait(metadata.capacity_wait_ms) }) : translateApp("{{count}} attempts", { count: metadata.attempts })) : (metadata?.capacity_wait_ms ? translateApp("Waited {{duration}} for capacity", { duration: formatCapacityWait(metadata.capacity_wait_ms) }) : ""))}
            </Label>
          ) : null}
          {delivery ? <DeliveryNotice delivery={delivery} /> : null}
        </View>
      ) : null}
    </View>
  );
}

/** "api-github__get_repo" -> "api-github"; NyxID's own tools have none. */
function serviceSlug(label: string): string | undefined {
  if (!label.includes("__")) return undefined;
  const slug = label.split("__")[0]!;
  return slug === "nyx" || slug === "nyxid" ? undefined : slug;
}

function ActivityStatusIcon({
  status,
}: {
  status: Activity["status"] | "confirmation" | "skipped";
}) {
  useAppLanguage();
  const { colors } = useTheme();
  return (
    <View
      style={styles.status}
      accessibilityElementsHidden
      importantForAccessibility="no-hide-descendants"
    >
      {status === "skipped" ? (
        <Minus size={14} color={colors.textSecondary} />
      ) : status === "unknown" ? (
        <CircleAlert size={14} color={colors.textSecondary} />
      ) : status === "running" ? (
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
  useAppLanguage();
  return (
    <Label muted style={styles.description}>
      {deliveryText(delivery)}
    </Label>
  );
}

const styles = StyleSheet.create({
  // A slim full-width row that reads as tappable (tint and a trailing
  // chevron); details open beneath it on a thin left rule.
  container: { gap: 6 },
  summary: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: TOUCH_TARGET - 6,
    paddingHorizontal: 12,
    paddingVertical: 4,
    borderRadius: radius.md + 2,
  },
  count: { fontSize: 13, lineHeight: 18, flex: 1 },
  status: {
    width: 20,
    height: 20,
    alignItems: "center",
    justifyContent: "center",
  },
  // Rows share the header's columns: status icon under its icon, text under
  // its label.
  details: { paddingHorizontal: 12, paddingVertical: 2, gap: 10 },
  activity: { flexDirection: "row", alignItems: "flex-start", gap: 8 },
  activityText: { flex: 1, gap: 2 },
  description: { fontSize: 12, lineHeight: 20 },
  name: { fontSize: 10, lineHeight: 16 },
});
