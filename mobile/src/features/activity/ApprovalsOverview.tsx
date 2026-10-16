import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  SectionList,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { ChevronRight, RefreshCw, ShieldCheck } from "lucide-react-native";
import { ChallengeCard } from "../../components/ChallengeCard";
import { HistoryCard } from "../../components/HistoryCard";
import type { ChallengeDetail } from "../../lib/api/types";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";

export type ApprovalListState = {
  items: readonly ChallengeDetail[];
  total?: number;
  isLoading: boolean;
  isError: boolean;
  hasNextPage: boolean;
  isFetchingNextPage: boolean;
  onRetry: () => void;
  onLoadMore: () => void;
};

/** Missing dates sort last instead of breaking the entire approval workspace. */
function compareDates(
  left: string | undefined,
  right: string | undefined,
  descending = false,
) {
  const a = Date.parse(left ?? "");
  const b = Date.parse(right ?? "");
  if (!Number.isFinite(a)) return Number.isFinite(b) ? 1 : 0;
  if (!Number.isFinite(b)) return -1;
  return descending ? b - a : a - b;
}

function historyTime(item: ChallengeDetail) {
  return item.decided_at && Number.isFinite(Date.parse(item.decided_at))
    ? item.decided_at
    : item.created_at;
}

export function ApprovalsOverview({
  pending,
  history,
  activeGrantCount,
  grantDurationLabel,
  mutatingIds,
  refreshing,
  pulling,
  offline,
  refreshDisabled,
  onRefresh,
  onPullRefresh,
  onOpenGrants,
  onOpenDetail,
  onDecide,
}: {
  pending: ApprovalListState;
  history: ApprovalListState;
  activeGrantCount?: number;
  grantDurationLabel: string;
  mutatingIds: ReadonlySet<string>;
  refreshing: boolean;
  pulling: boolean;
  offline: boolean;
  refreshDisabled: boolean;
  onRefresh: () => void;
  onPullRefresh: () => void;
  onOpenGrants: () => void;
  onOpenDetail: (item: ChallengeDetail, source: string) => void;
  onDecide: (item: ChallengeDetail, decision: "APPROVE" | "DENY") => void;
}) {
  const { colors } = useTheme();
  const styles = createStyles(colors);
  const sections = [
    {
      key: "pending",
      title: "Waiting on you",
      data: [...pending.items].sort((a, b) =>
        compareDates(a.expires_at, b.expires_at),
      ),
      state: pending,
    },
    {
      key: "history",
      title: "History",
      data: [...history.items].sort((a, b) =>
        compareDates(historyTime(a), historyTime(b), true),
      ),
      state: history,
    },
  ];

  return (
    <SectionList
      sections={sections}
      keyExtractor={(item) => item.id}
      stickySectionHeadersEnabled={false}
      contentContainerStyle={styles.content}
      refreshControl={
        <RefreshControl
          refreshing={pulling}
          onRefresh={onPullRefresh}
          tintColor={colors.primary}
        />
      }
      ListHeaderComponent={
        <View style={styles.intro}>
          <View style={styles.introRow}>
            <Text style={[styles.description, styles.introCopy]}>
              Requests gated by your NyxID policy wait here for your decision.
              Decisions in Web, Telegram and mobile stay in sync.
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Refresh approvals"
              accessibilityState={{
                disabled: refreshDisabled,
                busy: refreshing,
              }}
              disabled={refreshDisabled}
              onPress={onRefresh}
              style={[styles.refresh, refreshDisabled && styles.disabled]}
            >
              {refreshing ? (
                <ActivityIndicator color={colors.primary} />
              ) : (
                <RefreshCw size={20} color={colors.primary} />
              )}
            </Pressable>
          </View>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="View active grants"
            onPress={onOpenGrants}
            style={styles.grantsLink}
          >
            <ShieldCheck size={16} color={colors.textSecondary} />
            <Text style={styles.grantsLabel}>
              Active grants
              {activeGrantCount == null ? "" : ` · ${activeGrantCount}`}
            </Text>
            <ChevronRight size={16} color={colors.textMuted} />
          </Pressable>
        </View>
      }
      renderSectionHeader={({ section }) => (
        <View style={styles.sectionHeader}>
          <View style={styles.headingRow}>
            <Text accessibilityRole="header" style={styles.heading}>
              {section.title}
            </Text>
            {section.key === "pending" && section.state.total != null && (
              <Text style={styles.count}>{section.state.total}</Text>
            )}
          </View>
          {section.state.isError && (
            <View accessibilityRole="alert" style={styles.errorCard}>
              <Text style={styles.description}>
                Could not load{" "}
                {section.key === "pending"
                  ? "pending approvals"
                  : "approval history"}
                .
                {section.data.length
                  ? " Displayed requests may be out of date."
                  : " Retry to check for requests."}
              </Text>
              <Pressable
                accessibilityRole="button"
                accessibilityLabel={`Retry ${section.key === "pending" ? "pending approvals" : "approval history"}`}
                disabled={offline}
                onPress={section.state.onRetry}
                style={[styles.textButton, offline && styles.disabled]}
              >
                <Text style={styles.linkText}>Retry</Text>
              </Pressable>
            </View>
          )}
          {offline && !section.data.length ? (
            <View style={styles.emptyCard}>
              <Text style={styles.emptyDescription}>
                Connect to check{" "}
                {section.key === "pending"
                  ? "pending approvals"
                  : "approval history"}
                .
              </Text>
            </View>
          ) : section.state.isLoading && !section.data.length ? (
            <ActivityIndicator
              accessibilityLabel={`Loading ${section.key === "pending" ? "pending approvals" : "approval history"}`}
              color={colors.primary}
              style={styles.loading}
            />
          ) : !section.data.length && !section.state.isError ? (
            <View style={styles.emptyCard}>
              {section.key === "pending" ? (
                <>
                  <ShieldCheck size={28} color={colors.success} />
                  <Text style={styles.emptyTitle}>Nothing waiting on you</Text>
                  <Text style={styles.emptyDescription}>
                    Approval requests appear here when an action needs your
                    sign-off.
                  </Text>
                </>
              ) : (
                <Text style={styles.emptyDescription}>
                  Decided approvals will show up here.
                </Text>
              )}
            </View>
          ) : null}
        </View>
      )}
      renderItem={({ item, section }) => (
        <View style={styles.item}>
          {section.key === "pending" ? (
            <ChallengeCard
              challenge={item}
              grantDurationLabel={grantDurationLabel}
              isMutating={mutatingIds.has(item.id)}
              decisionsDisabled={offline}
              onPress={() => onOpenDetail(item, "pending_list")}
              onApprove={() => onDecide(item, "APPROVE")}
              onDeny={() => onDecide(item, "DENY")}
            />
          ) : (
            <HistoryCard
              item={item}
              onPress={() => onOpenDetail(item, "history_list")}
            />
          )}
        </View>
      )}
      renderSectionFooter={({ section }) =>
        section.state.hasNextPage ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={
              section.key === "pending"
                ? "Load more pending approvals"
                : "Load older approvals"
            }
            accessibilityState={{
              disabled: offline || section.state.isFetchingNextPage,
            }}
            disabled={offline || section.state.isFetchingNextPage}
            onPress={section.state.onLoadMore}
            style={[styles.moreButton, offline && styles.disabled]}
          >
            {section.state.isFetchingNextPage ? (
              <ActivityIndicator color={colors.primary} />
            ) : (
              <Text style={styles.linkText}>
                {section.key === "pending"
                  ? "Load more requests"
                  : "Load older approvals"}
              </Text>
            )}
          </Pressable>
        ) : null
      }
    />
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    content: { paddingHorizontal: spacing.xxxl, paddingBottom: 48 },
    intro: { gap: spacing.sm, paddingTop: spacing.sm },
    introRow: {
      flexDirection: "row",
      alignItems: "flex-start",
      gap: spacing.lg,
    },
    description: { ...typeScale.label, color: c.textSecondary },
    introCopy: { flex: 1 },
    refresh: {
      width: 44,
      height: 44,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: c.ghostBg,
      alignItems: "center",
      justifyContent: "center",
    },
    grantsLink: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
      alignSelf: "flex-start",
      minHeight: 44,
    },
    grantsLabel: { ...typeScale.label, color: c.textSecondary },
    sectionHeader: {
      gap: spacing.lg,
      paddingTop: spacing.huge,
      paddingBottom: spacing.lg,
    },
    headingRow: { flexDirection: "row", alignItems: "center", gap: spacing.sm },
    heading: { ...typeScale.title, color: c.textPrimary },
    count: {
      ...typeScale.badge,
      color: c.warning,
      backgroundColor: c.cardSoft,
      borderRadius: radius.sm,
      paddingHorizontal: spacing.sm,
      paddingVertical: spacing.xs,
    },
    errorCard: {
      padding: spacing.lg,
      borderWidth: 1,
      borderColor: c.border,
      borderRadius: radius.lg,
      gap: spacing.sm,
    },
    emptyCard: {
      padding: spacing.huge,
      backgroundColor: c.card,
      borderWidth: 1,
      borderColor: c.borderSoft,
      borderRadius: radius.lg,
      alignItems: "center",
      gap: spacing.sm,
    },
    emptyTitle: { ...typeScale.bodyStrong, color: c.textPrimary },
    emptyDescription: {
      ...typeScale.label,
      color: c.textSecondary,
      textAlign: "center",
    },
    item: { marginBottom: spacing.lg },
    linkText: { ...typeScale.label, color: c.primary },
    textButton: {
      minHeight: 44,
      justifyContent: "center",
      alignItems: "flex-start",
    },
    moreButton: {
      minHeight: 44,
      alignItems: "center",
      justifyContent: "center",
      borderWidth: 1,
      borderColor: c.border,
      borderRadius: radius.md,
      marginTop: spacing.xs,
    },
    loading: { padding: spacing.xxl },
    disabled: { opacity: 0.45 },
  });
