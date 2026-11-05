import { ServiceIcon } from "../../components/ServiceIcon";
import { useEffect, useState, type ReactNode } from "react";
import { Pressable, StyleSheet, View } from "react-native";
import { ChevronDown, ChevronRight, History } from "lucide-react-native";
import { Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius, TOUCH_TARGET } from "../../theme/designTokens";
import { acknowledgementDecisionLabel } from "./assistantState";
import type { AssistantAcknowledgement } from "./assistantTypes";
import {
  AssistantReviewTarget,
  type RevealReviewTarget,
} from "./AssistantReviewTarget";

/** Pending requests stay actionable; settled cards become optional history. */
export function AssistantAcknowledgements({
  acknowledgements,
  selectedId,
  renderPending,
  renderAuthoredReview,
  onToggle,
  onOpenLink,
  active = true,
  scope = "confirmation-history",
  onReveal,
}: {
  acknowledgements: AssistantAcknowledgement[];
  selectedId?: string;
  renderPending: (row: AssistantAcknowledgement) => ReactNode;
  renderAuthoredReview: (row: AssistantAcknowledgement) => ReactNode;
  onToggle: () => void;
  onOpenLink?: (url: string) => void;
  active?: boolean;
  scope?: string;
  onReveal?: RevealReviewTarget;
}) {
  const { colors } = useTheme();
  const selected = acknowledgements.find((row) => row.id === selectedId);
  const [open, setOpen] = useState(false);
  const [expandedId, setExpandedId] = useState<string>();
  useEffect(() => {
    // A deep link may arrive before history. Opening the selected record must
    // not reopen a card when that same pending request becomes settled.
    if (selected && selected.status !== "pending") {
      setOpen(true);
      setExpandedId(selected.id);
    }
  }, [selectedId, selected?.id]);
  const records = acknowledgements.filter((row) => row.status !== "pending");
  const count = `${records.length} confirmation ${records.length === 1 ? "record" : "records"}`;
  const Chevron = open ? ChevronDown : ChevronRight;
  return (
    <>
      {acknowledgements
        .filter((row) => row.status === "pending")
        .map((row) => (
          <AssistantReviewTarget
            key={row.id}
            selected={row.id === selectedId}
            active={active}
            scope={`${scope}:${row.id}`}
            onReveal={onReveal}
          >
            {renderPending(row)}
          </AssistantReviewTarget>
        ))}
      {records.length ? (
        <View
          style={[
            styles.container,
            { borderColor: colors.border, backgroundColor: colors.cardSoft },
          ]}
        >
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={count}
            accessibilityHint={
              open ? "Hide confirmation history" : "Show confirmation history"
            }
            accessibilityState={{ expanded: open }}
            onPress={() => {
              onToggle();
              setOpen((value) => !value);
            }}
            style={({ pressed }) => [
              styles.row,
              { opacity: pressed ? 0.65 : 1 },
            ]}
          >
            <Chevron size={15} color={colors.textSecondary} />
            <History size={16} color={colors.textSecondary} />
            <Label muted style={styles.recordText}>
              {count}
            </Label>
          </Pressable>
          {open ? (
            <View style={[styles.records, { borderColor: colors.border }]}>
              {records.map((row) => {
                const expanded = expandedId === row.id;
                const RowChevron = expanded ? ChevronDown : ChevronRight;
                const label = acknowledgementDecisionLabel(row);
                return (
                  <AssistantReviewTarget
                    key={row.id}
                    selected={row.id === selectedId}
                    active={active && open}
                    scope={`${scope}:${row.id}`}
                    onReveal={onReveal}
                  >
                    <Pressable
                      accessibilityRole="button"
                      accessibilityLabel={`${label} · ${row.summary}`}
                      accessibilityHint={
                        expanded
                          ? "Hide confirmation details"
                          : "Show confirmation details"
                      }
                      accessibilityState={{ expanded }}
                      onPress={() => {
                        onToggle();
                        setExpandedId(expanded ? undefined : row.id);
                      }}
                      style={({ pressed }) => [
                        styles.record,
                        row.id === selectedId && {
                          backgroundColor: colors.card,
                        },
                        { opacity: pressed ? 0.65 : 1 },
                      ]}
                    >
                      <View style={styles.recordBody}>
                        <Label
                          numberOfLines={expanded ? undefined : 1}
                          style={styles.status}
                        >
                          {label}
                        </Label>
                        <Label
                          muted
                          numberOfLines={expanded ? undefined : 1}
                          style={styles.recordText}
                        >
                          {row.summary}
                        </Label>
                      </View>
                      <RowChevron size={16} color={colors.textSecondary} />
                    </Pressable>
                    {expanded ? (
                      <View style={styles.details}>
                        {row.status === "used" ? (
                          <Label muted style={styles.recordText}>
                            This one-time confirmation has been used. Check the
                            assistant’s response for the action’s result.
                          </Label>
                        ) : null}
                        {row.service_name || row.service_slug ? (
                          <View
                            style={{
                              flexDirection: "row",
                              alignItems: "center",
                              gap: 8,
                            }}
                          >
                            <ServiceIcon
                              slug={row.service_slug ?? "custom"}
                              size="xs"
                            />
                            <Label
                              muted
                              style={[styles.recordText, { flex: 1 }]}
                            >
                              Service: {row.service_name ?? row.service_slug}
                              {row.service_name && row.service_slug
                                ? ` (${row.service_slug})`
                                : ""}
                            </Label>
                          </View>
                        ) : null}
                        {row.tool_name ? (
                          <Label muted mono style={styles.recordText}>
                            Tool: {row.tool_name}
                          </Label>
                        ) : null}
                        {row.confirmation ? (
                          <Label muted style={styles.recordText}>
                            Risk: {row.confirmation.risk}
                          </Label>
                        ) : null}
                        {row.decided_by ? (
                          <Label muted style={styles.recordText}>
                            Decided by{" "}
                            {row.decided_by === "user"
                              ? "you"
                              : row.decided_by === "orchestrator"
                                ? "NyxBot"
                                : row.decided_by === "access_mode"
                                  ? "Full access"
                                  : row.decided_by}
                          </Label>
                        ) : null}
                        {row.reason ? (
                          <Label muted style={styles.recordText}>
                            Decision reason: {row.reason}
                          </Label>
                        ) : null}
                        <Label muted style={styles.recordText}>
                          Requested: {formatDate(row.created_at)}
                        </Label>
                        <Label muted style={styles.recordText}>
                          Expiry: {formatDate(row.expires_at)}
                        </Label>
                        <Label muted mono style={styles.recordText}>
                          Confirmation: {row.id}
                        </Label>
                        {row.authored_skill ? renderAuthoredReview(row) : null}
                        {row.link && onOpenLink ? (
                          <Pressable
                            accessibilityRole="link"
                            accessibilityLabel="Inspect or discard proposal"
                            onPress={() => onOpenLink(row.link!)}
                            style={styles.record}
                          >
                            <Label muted>Inspect or discard proposal</Label>
                          </Pressable>
                        ) : null}
                      </View>
                    ) : null}
                  </AssistantReviewTarget>
                );
              })}
            </View>
          ) : null}
        </View>
      ) : null}
    </>
  );
}

function formatDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? "Unavailable" : date.toLocaleString();
}

const styles = StyleSheet.create({
  container: { borderWidth: 1, borderRadius: radius.md, overflow: "hidden" },
  row: {
    flexDirection: "row",
    alignItems: "center",
    gap: 8,
    minHeight: TOUCH_TARGET,
    paddingHorizontal: 10,
    paddingVertical: 6,
  },
  records: { borderTopWidth: 1, padding: 4, gap: 2 },
  record: {
    flexDirection: "row",
    alignItems: "center",
    minHeight: TOUCH_TARGET,
    gap: 8,
    padding: 10,
    borderRadius: radius.sm,
  },
  recordBody: { flex: 1, gap: 2 },
  recordText: { fontSize: 12, lineHeight: 18, flexShrink: 1 },
  status: { fontSize: 12, lineHeight: 18 },
  details: { paddingHorizontal: 10, paddingBottom: 12, gap: 6 },
});
