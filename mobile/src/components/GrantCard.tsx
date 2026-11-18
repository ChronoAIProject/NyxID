import { ServiceIcon } from "./ServiceIcon";
import { useMemo } from "react";
import { Pressable, StyleSheet, Text, View } from "react-native";
import { TOUCH_TARGET, spacing, typeScale } from "../theme/designTokens";
import { useTheme } from "../theme/ThemeContext";
import type { ThemeColors } from "../theme/mobileTheme";
import { GroupedCell } from "./GroupedCell";
import type { ApprovalItem } from "../lib/api/types";

type GrantCardProps = {
  grant: ApprovalItem;
  iconSlug?: string;
  iconUrl?: string | null;
  /** Position in the grants group, for the rounded corners and separators. */
  first?: boolean;
  last?: boolean;
  onRevoke?: () => void;
  isMutating?: boolean;
};

function formatDate(dateStr: string): string {
  const d = new Date(dateStr);
  return d.toLocaleDateString("en-US", { month: "short", day: "numeric" });
}

function daysUntilExpiry(expiresAt: string): number {
  return Math.round(
    (new Date(expiresAt).getTime() - Date.now()) / (24 * 60 * 60 * 1000),
  );
}

function formatExpiry(expiresAt: string): string {
  const days = daysUntilExpiry(expiresAt);
  if (days < 0) return "Expired";
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  return `${days}d left`;
}

/** An active grant: an inset-group row with its expiry and a red Revoke action. */
export function GrantCard({
  grant,
  iconSlug,
  iconUrl,
  first = true,
  last = true,
  onRevoke,
  isMutating = false,
}: GrantCardProps) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);

  const urgent = daysUntilExpiry(grant.expires_at) < 3;
  const expiryLabel = formatExpiry(grant.expires_at);
  const grantedLabel = `Approved ${formatDate(grant.granted_at)}`;
  const expiresLabel = `Expires ${formatDate(grant.expires_at)}`;
  const requesterLabel = grant.requester_label ?? grant.requester_type;
  const orgLabel =
    grant.org_scoped === true
      ? grant.org_name
        ? `Org · ${grant.org_name}`
        : "Org"
      : undefined;

  return (
    <GroupedCell first={first} last={last}>
      <View style={styles.row}>
        <View style={styles.tile}>
          <ServiceIcon
            slug={iconSlug ?? grant.service_id}
            iconUrl={iconUrl}
            size="md"
          />
        </View>
        <View style={styles.body}>
          <Text style={styles.title} numberOfLines={2}>
            {grant.service_name}
          </Text>
          {orgLabel ? <Text style={styles.secondary}>{orgLabel}</Text> : null}
          <Text style={styles.secondary} numberOfLines={1}>
            {grant.requester_type} · {requesterLabel}
          </Text>
          <Text style={styles.meta}>
            {grantedLabel} · {expiresLabel} ·{" "}
            <Text style={{ color: urgent ? colors.danger : colors.success }}>
              {expiryLabel}
            </Text>
          </Text>
        </View>
        <Pressable
          accessibilityRole="button"
          accessibilityLabel={`Revoke ${grant.service_name}`}
          accessibilityState={{ disabled: isMutating }}
          style={({ pressed }) => [
            styles.revoke,
            pressed && !isMutating && styles.pressed,
          ]}
          onPress={onRevoke}
          disabled={isMutating}
        >
          <Text
            style={[
              styles.revokeText,
              { color: isMutating ? colors.textMuted : colors.danger },
            ]}
          >
            Revoke
          </Text>
        </Pressable>
      </View>
    </GroupedCell>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    row: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.lg,
      minHeight: 52,
      paddingLeft: spacing.xxl,
      paddingRight: spacing.md,
      paddingVertical: spacing.md,
    },
    tile: {
      width: 30,
      alignSelf: "flex-start",
      alignItems: "center",
      paddingTop: 2,
    },
    body: { flex: 1, minWidth: 0, gap: 2 },
    title: { ...typeScale.title, color: c.textPrimary },
    secondary: { ...typeScale.label, color: c.textSecondary },
    meta: { ...typeScale.small, color: c.textMuted },
    revoke: {
      minHeight: TOUCH_TARGET,
      minWidth: TOUCH_TARGET,
      paddingHorizontal: spacing.md,
      alignItems: "center",
      justifyContent: "center",
      borderRadius: 8,
    },
    pressed: { backgroundColor: c.navActive },
    revokeText: { ...typeScale.label },
  });
