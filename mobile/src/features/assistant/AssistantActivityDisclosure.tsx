import { useState } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { Check, ChevronDown, ChevronRight, X } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius, TOUCH_TARGET } from "../../theme/designTokens";
import type { AssistantMessage } from "./assistantTypes";
import { describeToolActivity } from "./assistantActivityText";

type Activity = AssistantMessage["activities"][number];

export function AssistantActivityDisclosure({
  activities,
  onToggle,
}: {
  activities: AssistantMessage["activities"];
  onToggle: () => void;
}) {
  const { colors } = useTheme();
  const [open, setOpen] = useState(false);
  if (!activities.length) return null;
  const failed = activities.filter(
    (activity) => activity.status === "error",
  ).length;
  const status = activities.some((activity) => activity.status === "running")
    ? "running"
    : failed
      ? "error"
      : "completed";
  const count = `${activities.length} ${activities.length === 1 ? "action" : "actions"}`;
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
        accessibilityLabel={`${count}, ${status}${failed ? `, ${failed} failed` : ""}`}
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
          {count}
        </Label>
        {failed ? (
          <Label style={[styles.count, { color: colors.danger }]}>
            · {failed} failed
          </Label>
        ) : null}
      </Pressable>
      {open ? (
        <View style={[styles.details, { borderColor: colors.border }]}>
          {activities.map((activity) => {
            const description = describeToolActivity(
              activity.label,
              activity.status === "completed" ? "done" : activity.status,
            );
            return (
              <View
                key={activity.id}
                accessible
                accessibilityRole="text"
                accessibilityLabel={`${description}, ${activity.label}, ${activity.status}`}
                style={styles.activity}
              >
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
                  </Label>
                  <Label muted mono style={styles.name}>
                    {activity.label}
                  </Label>
                </View>
              </View>
            );
          })}
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
