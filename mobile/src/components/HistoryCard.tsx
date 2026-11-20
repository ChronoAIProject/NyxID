import { useAppLanguage } from "../i18n/LanguageProvider";
import { translateApp, appLocale } from "../i18n/appTranslation";
import { ServiceIcon } from "./ServiceIcon";
import { useMemo } from "react";
import { Text, Pressable, StyleSheet, View, useWindowDimensions } from "react-native";
import { ChevronRight } from "lucide-react-native";
import { spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { GroupedCell } from "./GroupedCell";
import type { ChallengeDetail, ChallengeStatus } from "../lib/api/types";

type HistoryCardProps = {
  item: ChallengeDetail;
  /** Position in its date group, for the rounded corners and separators. */
  first?: boolean;
  last?: boolean;
  onPress?: () => void;
};

function decisionColor(c: ThemeColors, status: ChallengeStatus): string {
  if (status === "APPROVED") return c.success;
  if (status === "DENIED") return c.danger;
  return c.textMuted;
}

// DESIGN.md §Interaction Rules: status badges always Title Case.
function decisionLabel(status: ChallengeStatus): string {
  if (status === "APPROVED") return translateApp("Approved");
  if (status === "DENIED") return translateApp("Denied");
  if (status === "PENDING") return translateApp("Pending");
  return translateApp("Expired");
}

function channelLabel(channel?: string | null): string | undefined {
  if (!channel?.trim()) return undefined;
  if (channel === "web") return translateApp("Web");
  if (channel === "telegram") return "Telegram";
  if (channel === "mobile" || channel === "push") return translateApp("Mobile");
  return channel;
}

function formatTime(dateStr: string): string | undefined {
  const d = new Date(dateStr);
  if (!Number.isFinite(d.getTime())) return undefined;
  return d.toLocaleString(appLocale(), {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "numeric",
    minute: "2-digit",
  });
}

function recordedRequester(item: ChallengeDetail): string | undefined {
  const requester = item.request_context.requester.trim();
  if (requester && requester !== "Unspecified" && requester !== "Unknown")
    return requester;
  const client = item.request_context.client.trim();
  return client && client !== "Unknown" && client !== "Unspecified"
    ? client
    : undefined;
}

/** A decided request: an inset-group row with its outcome as the trailing value. */
export function HistoryCard({
  item,
  first = true,
  last = true,
  onPress,
}: HistoryCardProps) {
  useAppLanguage();
  const { colors } = useTheme();
  const { width, fontScale } = useWindowDimensions();
  const compact = width / Math.max(1, fontScale) < 340;
  const styles = useMemo(() => createStyles(colors), [colors]);

  const summary =
    item.summary.trim() || `${item.action} ${item.resource}`.trim();
  const channel = channelLabel(item.decision_channel);
  const requester = recordedRequester(item);
  const decidedAt = item.decided_at ? formatTime(item.decided_at) : undefined;
  const requestedAt = formatTime(item.created_at);
  const riskLabel = translateApp(
    item.risk_level.charAt(0).toUpperCase() + item.risk_level.slice(1),
  );
  const decision = decisionLabel(item.status);

  const outcome = (
    <Text style={[styles.value, { color: decisionColor(colors, item.status) }]}>
      {decision}
    </Text>
  );
  return (
    <GroupedCell first={first} last={last}>
      <Pressable
        style={({ pressed }) => [
          styles.row,
          pressed && onPress && styles.pressed,
        ]}
        onPress={onPress}
        accessibilityRole={onPress ? "button" : undefined}
        accessibilityLabel={`${decision}: ${summary}`}
        accessibilityHint={
          onPress ? translateApp("View approval request details") : undefined
        }
      >
        <View style={styles.tile}>
          <ServiceIcon slug={item.service_slug} size="md" />
        </View>
        <View style={styles.body}>
          <Text style={styles.title} numberOfLines={compact ? undefined : 2}>
            {item.title}
          </Text>
          <Text style={styles.secondary}>{summary}</Text>
          {requester ? (
            <Text style={styles.meta}>
              {translateApp("Requested by {{name}}", { name: requester })}</Text>
          ) : null}
          <Text style={styles.meta}>
            {(channel ? (decidedAt
              ? (item.risk_level !== "unknown" ? translateApp("via {{channel}} · Decided {{time}} · {{risk}} risk", { channel: channel, time: decidedAt, risk: riskLabel }) : translateApp("via {{channel}} · Decided {{time}}", { channel: channel, time: decidedAt })) : (requestedAt ? (item.risk_level !== "unknown" ? translateApp("via {{channel}} · Requested {{time}} · {{risk}} risk", { channel: channel, time: requestedAt, risk: riskLabel }) : translateApp("via {{channel}} · Requested {{time}}", { channel: channel, time: requestedAt })) : (item.risk_level !== "unknown" ? translateApp("via {{channel}} · Time not recorded · {{risk}} risk", { channel: channel, risk: riskLabel }) : translateApp("via {{channel}} · Time not recorded", { channel: channel })))) : (decidedAt ? (item.risk_level !== "unknown" ? translateApp("Decided {{time}} · {{risk}} risk", { time: decidedAt, risk: riskLabel }) : translateApp("Decided {{time}}", { time: decidedAt })) : (requestedAt ? (item.risk_level !== "unknown" ? translateApp("Requested {{time}} · {{risk}} risk", { time: requestedAt, risk: riskLabel }) : translateApp("Requested {{time}}", { time: requestedAt })) : (item.risk_level !== "unknown" ? translateApp("Time not recorded · {{risk}} risk", { risk: riskLabel }) : translateApp("Time not recorded")))))}
          </Text>
          {compact && outcome}
        </View>
        {!compact && outcome}
        {onPress ? <ChevronRight size={18} color={colors.textMuted} /> : null}
      </Pressable>
    </GroupedCell>
  );
}

/** A date group's title, in the small caps of an iOS grouped list. */
export function HistorySectionHeader({ title }: { title: string }) {
  useAppLanguage();
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  return (
    <Text accessibilityRole="header" style={styles.sectionHeader}>
      {title.toUpperCase()}
    </Text>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    row: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.lg,
      minHeight: 52,
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.md,
    },
    pressed: { backgroundColor: c.navActive },
    tile: {
      width: 30,
      alignSelf: "flex-start",
      alignItems: "center",
      paddingTop: 2,
    },
    body: { flex: 1, minWidth: 0, gap: 2 },
    // DESIGN.md: Activity rows lead with the service name at 15px.
    title: { ...typeScale.title, color: c.textPrimary },
    secondary: { ...typeScale.label, color: c.textSecondary },
    meta: { ...typeScale.small, color: c.textMuted },
    value: { ...typeScale.label, flexShrink: 0 },
    sectionHeader: {
      ...typeScale.small,
      letterSpacing: 0.6,
      color: c.textMuted,
      paddingHorizontal: spacing.xxl,
    },
  });
