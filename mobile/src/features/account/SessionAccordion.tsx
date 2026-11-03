import { Pressable, StyleSheet, View } from "react-native";
import { ChevronDown, ChevronUp } from "lucide-react-native";
import { ActionButton, Label } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { radius, spacing } from "../../theme/designTokens";
import { sessionDeviceLabel } from "./sessionDevice";

type Session = {
  id: string;
  is_current?: boolean;
  user_agent?: string | null;
  ip_address?: string | null;
  created_at: string;
  expires_at: string;
};

/** One signed-in session: a compact summary that expands to its details and sign-out. */
export function SessionAccordion({
  session,
  expanded,
  onToggle,
  formatDate,
  onSignOut,
  signOutDisabled,
  signingOut,
  first = false,
}: {
  session: Session;
  expanded: boolean;
  onToggle: () => void;
  formatDate: (value: string) => string;
  onSignOut: () => void;
  signOutDisabled: boolean;
  signingOut: boolean;
  /** The first row sits flush with its card, so it draws no divider. */
  first?: boolean;
}) {
  const { colors } = useTheme();
  const device = sessionDeviceLabel(session.user_agent);
  const Chevron = expanded ? ChevronUp : ChevronDown;
  return (
    <View
      style={[
        styles.item,
        { borderTopColor: colors.border },
        first && { borderTopWidth: 0 },
      ]}
    >
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={`${device}${session.is_current ? ", this session" : ""}, signed in ${formatDate(session.created_at)}`}
        accessibilityHint={
          expanded ? "Hides session details" : "Shows session details"
        }
        accessibilityState={{ expanded }}
        onPress={onToggle}
        style={({ pressed }) => [styles.header, { opacity: pressed ? 0.7 : 1 }]}
      >
        <View style={styles.summary}>
          <View style={styles.titleRow}>
            <Label>{device}</Label>
            {session.is_current ? (
              <View
                style={[
                  styles.badge,
                  {
                    backgroundColor: colors.successTone.bg,
                    borderColor: colors.successTone.border,
                  },
                ]}
              >
                <Label style={{ color: colors.successTone.text, fontSize: 11 }}>
                  This session
                </Label>
              </View>
            ) : null}
          </View>
          <Label muted>Signed in {formatDate(session.created_at)}</Label>
        </View>
        <Chevron size={18} color={colors.textMuted} />
      </Pressable>
      {expanded ? (
        <View style={styles.details}>
          <Label muted selectable>
            {session.user_agent || "Device details unavailable"}
          </Label>
          <Label muted>Network: {session.ip_address || "Unavailable"}</Label>
          <Label muted>Expires {formatDate(session.expires_at)}</Label>
          <ActionButton
            label={
              session.is_current
                ? "Sign out on this device"
                : "Sign out session"
            }
            variant="danger"
            disabled={signOutDisabled}
            loading={signingOut}
            onPress={onSignOut}
          />
        </View>
      ) : null}
    </View>
  );
}

const styles = StyleSheet.create({
  item: { borderTopWidth: 1 },
  header: {
    minHeight: 56,
    flexDirection: "row",
    alignItems: "center",
    gap: spacing.lg,
    paddingVertical: spacing.md,
  },
  summary: { flex: 1, minWidth: 0, gap: 2 },
  titleRow: {
    flexDirection: "row",
    flexWrap: "wrap",
    alignItems: "center",
    gap: spacing.sm,
  },
  badge: {
    borderWidth: 1,
    borderRadius: radius.sm,
    paddingHorizontal: spacing.sm,
    paddingVertical: 1,
  },
  details: { gap: spacing.sm, paddingBottom: spacing.lg },
});
