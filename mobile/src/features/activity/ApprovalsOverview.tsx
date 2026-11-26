import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp, appLocale } from "../../i18n/appTranslation";
import {
  ActivityIndicator,
  Pressable,
  RefreshControl,
  SectionList,
  StyleSheet,
  Text,
  View,
} from "react-native";
import { RefreshCw, ShieldCheck } from "lucide-react-native";
import { ChallengeCard } from "../../components/ChallengeCard";
import { GroupedCell } from "../../components/GroupedCell";
import { SkeletonRows } from "../../components/Skeleton";
import {
  HistoryCard,
  HistorySectionHeader,
} from "../../components/HistoryCard";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import type { ChallengeDetail } from "../../lib/api/types";
import { spacing, typeScale } from "../../theme/designTokens";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { useRailScrollIndicator } from "../../components/workspace/workspaceFloatingInset";

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
      ? translateApp("Date not recorded")
      : key === now.toDateString()
        ? translateApp("Today")
        : key === yesterday.toDateString()
          ? translateApp("Yesterday")
          : date.toLocaleDateString(appLocale(), {
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
  const railIndicator = useRailScrollIndicator();
  useAppLanguage();
  const { colors } = useTheme();
  const styles = createStyles(colors);
  const listName = (key: string) =>
    key === "pending" ? translateApp("pending approvals")
      : translateApp("approval history");
  const dates = activityHistorySections(history.items);
  const sections = [
    {
      key: "pending",
      title: translateApp("Incoming"),
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
        title: translateApp("History"),
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
              {translateApp(
                "Requests gated by your NyxID policy wait here for your decision. Decisions in Web, Telegram and mobile stay in sync.",
              )}
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel={translateApp("Refresh activity")}
              accessibilityState={{
                disabled: refreshDisabled,
                busy: refreshing,
              }}
              disabled={refreshDisabled}
              hitSlop={4}
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
          <SettingsGroup>
            <SettingsRow
              icon={ShieldCheck}
              label={translateApp("Active grants")}
              value={
                activeGrantCount == null ? undefined : String(activeGrantCount)
              }
              accessibilityHint={translateApp("Opens your active grants")}
              onPress={onOpenGrants}
            />
          </SettingsGroup>
        </View>
      }
      renderSectionHeader={({ section }) =>
        section.key.startsWith("history:") ? (
          <View style={styles.dateHeader}>
            <HistorySectionHeader title={section.historyDate!} />
          </View>
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
              <View accessibilityRole="alert" style={styles.errorNote}>
                <Text style={styles.description}>{(section.data.length ? translateApp("Could not load {{name}}. Displayed requests may be out of date.", { name: listName(section.key)}) : translateApp("Could not load {{name}}. Retry to check for requests.", { name: listName(section.key) }))}
                </Text>
                <Pressable
                  accessibilityRole="button"
                  accessibilityLabel={translateApp("Retry {{listName}}", {
                    listName: listName(section.key),
                  })}
                  disabled={offline}
                  onPress={section.state.onRetry}
                  style={[styles.textButton, offline && styles.disabled]}
                >
                  <Text style={styles.linkText}>{translateApp("Retry")}</Text>
                </Pressable>
              </View>
            )}
            {offline && !section.data.length ? (
              <GroupedCell first last inset={false}>
                <Text style={[styles.emptyDescription, styles.emptyRow]}>
                  {translateApp("Connect to check {{list}}.", {
                    list: listName(section.key)})}
                </Text>
              </GroupedCell>
            ) : section.state.isLoading && !section.data.length ? (
              <SkeletonRows
                rows={section.key === "pending" ? 2 : 3}
                accessibilityLabel={translateApp("Loading {{listName}}", {
                  listName: listName(section.key),
                })}
              />
            ) : !section.data.length && !section.state.isError ? (
              <GroupedCell first last inset={false}>
                {section.key === "pending" ? (
                  <View style={styles.emptyRow}>
                    <ShieldCheck size={28} color={colors.success} />
                    <Text style={styles.emptyTitle}>
                      {translateApp("Nothing waiting on you")}
                    </Text>
                    <Text style={styles.emptyDescription}>
                      {translateApp(
                        "Approval requests appear here when an action needs your sign-off.",
                      )}
                    </Text>
                  </View>
                ) : (
                  <Text style={[styles.emptyDescription, styles.emptyRow]}>
                    {translateApp("Decided approvals will show up here.")}
                  </Text>
                )}
              </GroupedCell>
            ) : null}
            {section.historyDate ? (
              <HistorySectionHeader title={section.historyDate} />
            ) : null}
          </View>
        )
      }
      renderItem={({ item, section, index = 0 }) => {
        const first = index === 0;
        const last = index === section.data.length - 1;
        return section.key === "pending" ? (
          <ChallengeCard
            challenge={item}
            first={first}
            last={last}
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
            first={first}
            last={last}
            onPress={() => onOpenDetail(item, "history_list")}
          />
        );
      }}
      renderSectionFooter={({ section }) =>
        section.state.hasNextPage &&
        (section.key === "pending" || section.lastHistory) ? (
          <Pressable
            accessibilityRole="button"
            accessibilityLabel={
              section.key === "pending"
                ? translateApp("Load more pending approvals")
                : translateApp("Load older approvals")
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
                  ? translateApp("Load More Requests")
                  : translateApp("Load Older Approvals")}
              </Text>
            )}
          </Pressable>
        ) : null
      }
      {...railIndicator}
    />
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    content: {
      paddingHorizontal: spacing.xxl,
      paddingTop: 16,
      paddingBottom: 48,
    },
    intro: { gap: spacing.lg, paddingTop: spacing.sm },
    introRow: {
      flexDirection: "row",
      alignItems: "flex-start",
      gap: spacing.sm,
      paddingHorizontal: spacing.xxl,
    },
    description: { ...typeScale.label, color: c.textSecondary },
    introCopy: { flex: 1, paddingTop: spacing.xs },
    // Borderless 44pt icon target; the tint carries the affordance.
    refresh: {
      width: 44,
      height: 44,
      marginTop: -spacing.md,
      marginRight: -spacing.md,
      alignItems: "center",
      justifyContent: "center",
    },
    sectionHeader: {
      gap: spacing.md,
      paddingTop: spacing.huge,
      paddingBottom: spacing.sm,
    },
    headingRow: {
      flexDirection: "row",
      alignItems: "center",
      gap: spacing.sm,
      paddingHorizontal: spacing.xxl,
    },
    heading: { ...typeScale.title, color: c.textPrimary },
    count: { ...typeScale.label, color: c.warning },
    dateHeader: { paddingTop: spacing.xxl, paddingBottom: spacing.sm },
    errorNote: { gap: 2, paddingHorizontal: spacing.xxl },
    emptyRow: {
      paddingHorizontal: spacing.xxl,
      paddingVertical: spacing.xxl,
      alignItems: "center",
      gap: spacing.sm,
    },
    emptyTitle: { ...typeScale.bodyStrong, color: c.textPrimary },
    emptyDescription: {
      ...typeScale.label,
      color: c.textSecondary,
      textAlign: "center",
    },
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
      marginTop: spacing.xs,
    },
    disabled: { opacity: 0.45 },
  });
