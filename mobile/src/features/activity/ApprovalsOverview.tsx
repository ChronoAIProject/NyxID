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
import {
  HistoryCard,
  HistorySectionHeader,
} from "../../components/HistoryCard";
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

/** Main's local-day grouping, with a stable fallback for malformed legacy rows. */
export function activityHistorySections(
  items: readonly ChallengeDetail[],
  now = new Date(),
) {
  const yesterday = new Date(now);
  yesterday.setDate(yesterday.getDate() - 1);
  const groups = new Map<string, { title: string; data: ChallengeDetail[] }>();
  for (const item of [...items].sort((a, b) =>
    compareDates(a.created_at, b.created_at, true),
  )) {
    const date = new Date(item.created_at);
    const valid = Number.isFinite(date.getTime());
    const key = valid ? date.toDateString() : "unknown";
    const title = !valid
      ? "Date not recorded"
      : key === now.toDateString()
        ? "Today"
        : key === yesterday.toDateString()
          ? "Yesterday"
          : date.toLocaleDateString("en-US", {
              month: "short",
              day: "numeric",
            });
    if (!groups.has(key)) groups.set(key, { title, data: [] });
    groups.get(key)!.data.push(item);
  }
  return [...groups.values()];
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
  const dates = activityHistorySections(history.items);
  const sections = [
    {
      key: "pending",
      title: "Incoming",
      historyDate: undefined,
      lastHistory: false,
      data: [...pending.items].sort((a, b) =>
        compareDates(a.expires_at, b.expires_at),
      ),
      state: pending,
    },
    ...(dates.length ? dates : [{ title: undefined, data: [] }]).map(
      (group, index, all) => ({
        key: index === 0 ? "history" : `history:${index}`,
        title: "History",
        historyDate: group.title,
        data: group.data,
        state: history,
        lastHistory: index === all.length - 1,
      }),
    ),
  ];

  return (
    <SectionList
      sections={sections}
      keyExtractor={(item) => item.id}
      stickySectionHeadersEnabled={false}
      contentInsetAdjustmentBehavior="never"
      automaticallyAdjustContentInsets={false}
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
              accessibilityLabel="Refresh activity"
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
      renderSectionHeader={({ section }) =>
        section.key.startsWith("history:") ? (
          <HistorySectionHeader title={section.historyDate!} />
        ) : (
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
                    <Text style={styles.emptyTitle}>
                      Nothing waiting on you
                    </Text>
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
            {section.historyDate ? (
              <HistorySectionHeader title={section.historyDate} />
            ) : null}
          </View>
        )
      }
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
        section.state.hasNextPage &&
        (section.key === "pending" || section.lastHistory) ? (
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
    content: {
      paddingHorizontal: spacing.xxxl,
      paddingTop: 16,
      paddingBottom: 48,
    },
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
    linkText: { ...typeScale.label, color: c.primaryOnTint },
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
