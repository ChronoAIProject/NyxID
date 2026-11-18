import { useEffect, useState } from "react";
import { ActivityIndicator, StyleSheet, View } from "react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import type { AssistantConversation } from "./assistantTypes";
import type { AssistantLiveReply } from "./assistantStream";
import { describeToolActivity } from "./assistantActivityText";
import {
  CAPACITY_ACKNOWLEDGEMENT,
  deriveTurnStatus,
  elapsedLabel,
  formatElapsed,
} from "./assistantTurnPresentation";

export function AssistantTurnStatus({
  turn,
  live,
  pendingApproval,
  interrupted,
}: {
  turn?: AssistantConversation["active_turn"];
  live?: AssistantLiveReply;
  pendingApproval: boolean;
  interrupted: boolean;
}) {
  const { colors } = useTheme();
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const current =
    turn ??
    (live?.turnId
      ? {
          started_at: live.status?.started_at,
          continuations: live.continuations,
        }
      : undefined);
  const status = deriveTurnStatus({
    activeTurn: current,
    live: !!live && !live.terminal,
    pendingApproval,
    retrying: live?.noticeCode === "retrying",
    interrupted,
  });
  const queued = turn?.queued ?? live?.status?.queued;
  const capacity =
    turn?.waiting_for_capacity ?? live?.status?.waiting_for_capacity;
  const tool = turn?.activities.find(
    (activity) => activity.status === "running",
  );
  const label = turn?.stop_requested
    ? "Stopping"
    : capacity
      ? queued
        ? "Queued until another reply finishes"
        : "Waiting for the assistant"
      : status?.phase === "tool" && tool
        ? `${describeToolActivity(tool.label, "running")}…`
        : (status?.label ?? "Thinking");
  const elapsed = elapsedLabel(status?.startedAt, undefined, now);
  const from = Date.parse(
    turn?.capacity_wait_started_at ??
      live?.status?.capacity_wait_started_at ??
      "",
  );
  const to = Date.parse(
    turn?.capacity_wait_deadline ?? live?.status?.capacity_wait_deadline ?? "",
  );
  const bound = Math.max(0, Math.ceil((to - from) / 1000));
  const waited = Math.min(bound, Math.max(0, Math.floor((now - from) / 1000)));
  return (
    <View style={styles.container}>
      {capacity ? <Label>{CAPACITY_ACKNOWLEDGEMENT}</Label> : null}
      <View
        style={styles.row}
        accessibilityRole="text"
        accessibilityLiveRegion="polite"
        accessibilityLabel={`${label}${interrupted ? ". Connection interrupted; checking every few seconds" : ""}`}
      >
        <ActivityIndicator size="small" color={colors.primary} />
        <Label muted style={styles.label}>
          {label}
          {interrupted
            ? " Connection interrupted; checking every few seconds"
            : ""}
          {capacity && Number.isFinite(bound)
            ? ` · ${formatElapsed(waited)} / ${formatElapsed(bound)}`
            : elapsed
              ? ` · ${elapsed} total`
              : ""}
        </Label>
      </View>
      {live?.notice &&
      live.noticeCode !== "retrying" &&
      live.noticeCode !== "context_reset" ? (
        <Label muted>{live.notice}</Label>
      ) : null}
    </View>
  );
}
const styles = StyleSheet.create({
  container: { gap: 6 },
  row: { flexDirection: "row", alignItems: "center", gap: 8, minHeight: 28 },
  label: { fontSize: 12, lineHeight: 18, flex: 1 },
});
