import type { ServiceConnection } from "../services/serviceContracts";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { capture } from "../../lib/telemetry";
import {
  ActivityIndicator,
  Alert,
  AppState,
  FlatList,
  Pressable,
  RefreshControl,
  StyleSheet,
  Text,
  View,
} from "react-native";
import {
  useFocusEffect,
  useNavigation,
  useRoute,
  type RouteProp,
} from "@react-navigation/native";
import {
  onlineManager,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ScreenContainer } from "../../components/ScreenContainer";

import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { GrantCard } from "../../components/GrantCard";
import { OfflineBanner } from "../../components/OfflineBanner";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import {
  getDecisionErrorMessage,
  formatGrantDuration,
} from "./challengeUiState";
import { PrimaryButton } from "../../components/PrimaryButton";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import type {
  WorkspaceNavigationProp,
  WorkspaceContentParamList,
} from "../../app/navigationTypes";
import type { ActivitySegment } from "./activityTypes";
import type {
  ApprovalMode,
  ChallengeDetail,
  ApprovalItem,
} from "../../lib/api/types";
import {
  APPROVAL_BACKSTOP_POLL_INTERVAL_MS,
  signalApprovalStateMayHaveChanged,
  subscribeToApprovalRefreshSignals,
} from "../../lib/notifications/approvalRefreshSignal";
import { RefreshCw } from "lucide-react-native";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { ApprovalsOverview } from "./ApprovalsOverview";

type Nav = WorkspaceNavigationProp;

export function ActivityScreen() {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const navigation = useNavigation<Nav>();
  const route =
    useRoute<
      RouteProp<WorkspaceContentParamList, "Activity" | "ActiveGrants">
    >();
  const queryClient = useQueryClient();
  const { isConnected, recheckConnection } = useNetworkStatus();
  const [isLiveRefreshEnabled, setIsLiveRefreshEnabled] = useState(false);
  const activeSegment: ActivitySegment =
    route.name === "ActiveGrants" ? "active" : "pending";
  const setActiveSegment = useCallback(
    (segment: ActivitySegment) => {
      if (segment === "active" && route.name !== "ActiveGrants")
        navigation.push("ActiveGrants");
      else if (segment !== "active" && route.name === "ActiveGrants")
        navigation.goBack();
    },
    [navigation, route.name],
  );
  const activeSegmentRef = useRef(activeSegment);
  activeSegmentRef.current = activeSegment;
  const [isPullRefreshing, setIsPullRefreshing] = useState(false);
  const [isRefreshRunning, setIsRefreshRunning] = useState(false);
  const [isUserRefreshRunning, setIsUserRefreshRunning] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [mutatingIds, setMutatingIds] = useState<Set<string>>(new Set());
  useWorkspaceBack({
    active: false,
    defaultRole: route.name === "ActiveGrants" ? "back" : "menu",
    disabled: mutatingIds.size > 0,
    onBack: navigation.goBack,
  });
  const listPendingShownAtRef = useRef<number | null>(null);
  const openApprovalDetail = useCallback(
    (challenge: ChallengeDetail, entryPoint: string) => {
      queryClient.setQueryData(["challenge", challenge.id], challenge);
      navigation.push("ActivityDetail", {
        challengeId: challenge.id,
        entryPoint,
      });
    },
    [navigation, queryClient],
  );

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(t);
  }, [toast]);

  // --- Queries ---
  const PAGE_SIZE = 20;

  const getNextPageParam = (lastPage: {
    total: number;
    page: number;
    per_page: number;
  }) => {
    const totalPages = Math.ceil(lastPage.total / lastPage.per_page);
    return lastPage.page < totalPages ? lastPage.page + 1 : undefined;
  };

  const pendingQuery = useInfiniteQuery({
    queryKey: ["challenges", "pending"],
    queryFn: ({ pageParam }) => mobileApi.getChallenges(pageParam, PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam,
    refetchInterval: isLiveRefreshEnabled
      ? APPROVAL_BACKSTOP_POLL_INTERVAL_MS
      : false,
  });

  const approvalsQuery = useInfiniteQuery({
    queryKey: ["approvals"],
    queryFn: ({ pageParam }) => mobileApi.getApprovals(pageParam, PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam,
    refetchInterval: isLiveRefreshEnabled
      ? APPROVAL_BACKSTOP_POLL_INTERVAL_MS
      : false,
  });

  const settingsQuery = useQuery({
    queryKey: ["notifications", "settings"],
    queryFn: mobileApi.getNotificationSettings,
  });

  const historyQuery = useInfiniteQuery({
    queryKey: ["challenges", "history"],
    queryFn: ({ pageParam }) => mobileApi.getHistory(pageParam, PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam,
    refetchInterval: isLiveRefreshEnabled
      ? APPROVAL_BACKSTOP_POLL_INTERVAL_MS
      : false,
  });

  const pendingItems = pendingQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const activeItems = approvalsQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const historyItems = historyQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const grantDurationLabel = formatGrantDuration(
    settingsQuery.data?.grant_expiry_days,
  );

  const pendingCount = pendingItems.length;

  // Stamp the moment the pending list first has items under an active
  // "pending" segment. Used as the decision-latency baseline for
  // inline approve/deny taps (no detail sheet). Cleared when the user
  // leaves the pending segment or the list empties, so the next visit
  // measures its own view duration.
  useEffect(() => {
    if (activeSegment === "pending" && pendingCount > 0) {
      if (listPendingShownAtRef.current == null) {
        listPendingShownAtRef.current = Date.now();
      }
    } else {
      listPendingShownAtRef.current = null;
    }
  }, [activeSegment, pendingCount]);

  // The same native detail route handles list taps and cold/push identifiers.
  const deepLinkChallengeId = route.params?.challengeId;
  useEffect(() => {
    if (!deepLinkChallengeId) return;
    navigation.setParams({ challengeId: undefined });
    navigation.push("ActivityDetail", {
      challengeId: deepLinkChallengeId,
      entryPoint: "deep_link",
    });
  }, [deepLinkChallengeId, navigation]);

  // --- Mutations ---
  const decideMutation = useMutation({
    mutationFn: async ({
      id,
      decision,
    }: {
      id: string;
      decision: "APPROVE" | "DENY";
      approvalMode: ApprovalMode;
    }) => {
      const durationSec =
        decision === "APPROVE"
          ? (settingsQuery.data?.grant_expiry_days ?? 30) * 86400
          : undefined;
      return mobileApi.submitDecision(id, decision, durationSec);
    },
    onMutate: ({ id }) => {
      // Inline decisions measure from the pending inbox becoming visible.
      const openedAt = listPendingShownAtRef.current;
      const decisionMs =
        openedAt != null ? Math.max(0, Date.now() - openedAt) : 0;
      setMutatingIds((prev) => new Set(prev).add(id));
      return { decisionMs };
    },
    onSuccess: (_, { decision, approvalMode }, context) => {
      capture({
        name: "ui.mobile_decision_made",
        props: {
          domain: "approvals",
          decision: decision === "APPROVE" ? "approve" : "deny",
          decision_ms: context?.decisionMs ?? 0,
        },
      });
      void queryClient.invalidateQueries({ queryKey: ["challenges"] });
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });

      const isGrant = decision === "APPROVE" && approvalMode === "grant";
      setActiveSegment("pending");
      setToast({
        message: decision === "APPROVE" ? "Approved" : "Denied",
        kind: "success",
        ...(isGrant
          ? {
              action: {
                label: "View grants",
                onPress: () => setActiveSegment("active"),
              },
            }
          : {}),
      });
    },
    onError: (error, { id }) => {
      setToast({ message: getDecisionErrorMessage(error), kind: "error" });
    },
    onSettled: (_, __, { id }) => {
      setMutatingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    },
  });

  const revokeMutation = useMutation({
    mutationFn: ({ id, orgId }: { id: string; orgId?: string | null }) =>
      mobileApi.revoke(id, orgId),
    onMutate: ({ id }) => {
      setMutatingIds((prev) => new Set(prev).add(id));
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
      setToast({ message: "Revoked", kind: "success" });
    },
    onError: () => {
      setToast({ message: "Failed to revoke. Try again.", kind: "error" });
    },
    onSettled: (_, __, { id }) => {
      setMutatingIds((prev) => {
        const next = new Set(prev);
        next.delete(id);
        return next;
      });
    },
  });

  const handleRevoke = useCallback(
    (grant: ApprovalItem) => {
      Alert.alert("Revoke Access", `Revoke access for ${grant.service_name}?`, [
        { text: "Cancel", style: "cancel" },
        {
          text: "Revoke",
          style: "destructive",
          // Forward org_id when the grant is org-scoped so the backend
          // pivots ownership to the owning org (otherwise DELETE 404s
          // because the default path searches by user_id = actor).
          onPress: () => {
            capture({
              name: "ui.mobile_destructive_confirmed",
              props: { domain: "approvals", action: "revoke_session" },
            });
            revokeMutation.mutate({
              id: grant.id,
              orgId: grant.org_id ?? null,
            });
          },
        },
      ]);
    },
    [revokeMutation],
  );

  const refreshInFlightRef = useRef<Promise<void> | null>(null);
  const queuedSignalRefreshRef = useRef(false);
  const acceptsSignalRefreshRef = useRef(false);

  const returnQueuedSignalToBus = useCallback(() => {
    if (!queuedSignalRefreshRef.current) return;
    queuedSignalRefreshRef.current = false;
    // The throttle already consumed this signal when it called us. If focus,
    // app activity, or connectivity disappears before the follow-up can run,
    // put the work back so the next eligible approval screen catches up.
    signalApprovalStateMayHaveChanged();
  }, []);

  const refreshVisibleSegmentAndCounts = useCallback(
    async (refreshAllSegments = false) => {
      // The inbox shows pending requests and history together. Refresh both
      // whenever it is visible, and keep the auxiliary grant count coherent.
      // Push signals refresh every list, including while grants are open.
      const refetches: Promise<unknown>[] = [
        pendingQuery.refetch({ cancelRefetch: false }),
        approvalsQuery.refetch({ cancelRefetch: false }),
      ];
      if (refreshAllSegments || activeSegmentRef.current !== "active") {
        refetches.push(historyQuery.refetch({ cancelRefetch: false }));
      }
      await Promise.all(refetches);
    },
    [pendingQuery.refetch, approvalsQuery.refetch, historyQuery.refetch],
  );

  const handleRefresh = useCallback(
    (source: "user" | "signal" = "user"): Promise<void> => {
      if (!onlineManager.isOnline() || AppState.currentState !== "active") {
        if (source === "signal") {
          // `deliver()` has already opened the throttle window, so this can
          // retry at most once per second. The AppState/NetInfo-driven focus
          // cleanup unsubscribes and cancels that timer when the condition
          // persists; re-signaling keeps the hint pending for the next focus.
          signalApprovalStateMayHaveChanged();
        }
        return Promise.resolve();
      }

      if (refreshInFlightRef.current) {
        // A manual double-tap simply joins the existing request. Signals are
        // different: queue exactly one follow-up so a state change that lands
        // during a slow request is not lost when that older response wins.
        if (source === "signal") {
          queuedSignalRefreshRef.current = true;
        }
        return refreshInFlightRef.current;
      }

      setIsRefreshRunning(true);
      const refreshPromise = (async () => {
        let shouldRefreshAllSegments = source === "signal";
        let shouldRunAgain = false;

        try {
          do {
            queuedSignalRefreshRef.current = false;
            await refreshVisibleSegmentAndCounts(shouldRefreshAllSegments);
            shouldRunAgain = queuedSignalRefreshRef.current;

            if (
              shouldRunAgain &&
              (!acceptsSignalRefreshRef.current ||
                !onlineManager.isOnline() ||
                AppState.currentState !== "active")
            ) {
              returnQueuedSignalToBus();
              shouldRunAgain = false;
            }

            // A second pass can only have been requested by a signal that
            // arrived during the first pass, so it must include History too.
            shouldRefreshAllSegments = shouldRunAgain;
          } while (shouldRunAgain);
        } finally {
          // Also preserve a follow-up if the underlying refetch rejects.
          returnQueuedSignalToBus();
        }
      })().finally(() => {
        refreshInFlightRef.current = null;
        setIsRefreshRunning(false);
      });

      refreshInFlightRef.current = refreshPromise;
      return refreshPromise;
    },
    [refreshVisibleSegmentAndCounts, returnQueuedSignalToBus],
  );

  useFocusEffect(
    useCallback(() => {
      let unsubscribeSignal: (() => void) | null = null;

      const stopSignalSubscription = () => {
        setIsLiveRefreshEnabled(false);
        acceptsSignalRefreshRef.current = false;
        unsubscribeSignal?.();
        unsubscribeSignal = null;
        returnQueuedSignalToBus();
      };

      const syncSubscription = (appState: string) => {
        const shouldSubscribe =
          appState === "active" && isConnected && onlineManager.isOnline();
        setIsLiveRefreshEnabled(shouldSubscribe);

        if (shouldSubscribe && !unsubscribeSignal) {
          acceptsSignalRefreshRef.current = true;
          unsubscribeSignal = subscribeToApprovalRefreshSignals(() => {
            void handleRefresh("signal");
          });

          // A detail route may have consumed the live signal while this list
          // was blurred. The push invalidation remains on the cache, so use it
          // as the durable indication that this focus needs to catch up.
          const hasInvalidatedApprovalData = [
            ["challenges", "pending"],
            ["approvals"],
            ["challenges", "history"],
          ].some(
            (queryKey) => queryClient.getQueryState(queryKey)?.isInvalidated,
          );
          if (hasInvalidatedApprovalData) {
            signalApprovalStateMayHaveChanged();
          }
          return;
        }

        if (!shouldSubscribe && unsubscribeSignal) {
          stopSignalSubscription();
        }
      };

      syncSubscription(AppState.currentState);
      const appStateSubscription = AppState.addEventListener(
        "change",
        syncSubscription,
      );

      return () => {
        stopSignalSubscription();
        appStateSubscription.remove();
      };
    }, [handleRefresh, isConnected, queryClient, returnQueuedSignalToBus]),
  );

  const handleHeaderRefresh = useCallback(async () => {
    // A tap during signal-driven work joins that request without disabling the
    // control. Only a user-started request owns the disabled state.
    const startedUserRefresh = refreshInFlightRef.current === null;
    if (startedUserRefresh) {
      setIsUserRefreshRunning(true);
    }
    try {
      await handleRefresh("user");
    } finally {
      if (startedUserRefresh) {
        setIsUserRefreshRunning(false);
      }
    }
  }, [handleRefresh]);

  const handlePullRefresh = useCallback(async () => {
    setIsPullRefreshing(true);
    try {
      await handleRefresh("user");
    } finally {
      setIsPullRefreshing(false);
    }
  }, [handleRefresh]);

  const handleOfflineRetry = useCallback(async () => {
    const online = await recheckConnection();
    if (online) {
      await handleRefresh("user");
    } else {
      setToast({
        message: "Still offline — will retry when connected",
        kind: "error",
      });
    }
  }, [recheckConnection, handleRefresh]);

  // Passive backstop polls deliberately stay invisible. Only refreshes started
  // by the user or approval signal own the header/pull refresh feedback.
  const isRefreshing = isRefreshRunning || isPullRefreshing;

  const sortedActiveItems = [...activeItems].sort(
    (a, b) =>
      new Date(a.expires_at).getTime() - new Date(b.expires_at).getTime(),
  );
  const isManualRefreshDisabled =
    !isConnected || isUserRefreshRunning || isPullRefreshing;

  return (
    <ScreenContainer>
      {!isConnected && <OfflineBanner onRetry={handleOfflineRetry} />}
      {activeSegment !== "active" ? (
        <ApprovalsOverview
          pending={{
            items: pendingItems,
            total: pendingQuery.data?.pages[0]?.total,
            isLoading: pendingQuery.isPending,
            isError: pendingQuery.isError,
            hasNextPage: Boolean(pendingQuery.hasNextPage),
            isFetchingNextPage: pendingQuery.isFetchingNextPage,
            onRetry: () => {
              void pendingQuery.refetch();
            },
            onLoadMore: () => {
              if (pendingQuery.hasNextPage && !pendingQuery.isFetchingNextPage)
                void pendingQuery.fetchNextPage();
            },
          }}
          history={{
            items: historyItems,
            isLoading: historyQuery.isPending,
            isError: historyQuery.isError,
            hasNextPage: Boolean(historyQuery.hasNextPage),
            isFetchingNextPage: historyQuery.isFetchingNextPage,
            onRetry: () => {
              void historyQuery.refetch();
            },
            onLoadMore: () => {
              if (historyQuery.hasNextPage && !historyQuery.isFetchingNextPage)
                void historyQuery.fetchNextPage();
            },
          }}
          activeGrantCount={approvalsQuery.data?.pages[0]?.total}
          grantDurationLabel={grantDurationLabel}
          mutatingIds={mutatingIds}
          refreshing={isRefreshing}
          pulling={isPullRefreshing}
          offline={!isConnected}
          refreshDisabled={isManualRefreshDisabled}
          onRefresh={() => {
            void handleHeaderRefresh();
          }}
          onPullRefresh={() => {
            void handlePullRefresh();
          }}
          onOpenGrants={() => setActiveSegment("active")}
          onOpenDetail={openApprovalDetail}
          onDecide={(item, decision) =>
            decideMutation.mutate({
              id: item.id,
              decision,
              approvalMode: item.approval_mode,
            })
          }
        />
      ) : (
        <>
          <View style={styles.header}>
            <Text style={[styles.subtitle, styles.headerCopy]}>
              Grant-mode approvals allow matching requests without another
              decision until the grant expires or is revoked.
            </Text>
            <Pressable
              accessibilityRole="button"
              accessibilityLabel="Refresh active grants"
              accessibilityState={{
                disabled: isManualRefreshDisabled,
                busy: isRefreshing,
              }}
              disabled={isManualRefreshDisabled}
              onPress={() => {
                void handleHeaderRefresh();
              }}
              style={styles.refreshButton}
            >
              {isRefreshing ? (
                <ActivityIndicator color={colors.primary} />
              ) : (
                <RefreshCw size={21} color={colors.primary} />
              )}
            </Pressable>
          </View>
          <FlatList
            data={sortedActiveItems}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => {
              const loaded = [
                ...(queryClient.getQueryData<ServiceConnection[]>([
                  "workspace",
                  "services",
                  "connections",
                ]) ?? []),
                ...(queryClient.getQueryData<ServiceConnection[]>([
                  "workspace",
                  "services",
                  "plugins",
                ]) ?? []),
              ];
              const connection = loaded.find(
                (row) =>
                  row.id === item.service_id ||
                  row.catalog_service_id === item.service_id,
              );
              return (
                <GrantCard
                  grant={item}
                  iconSlug={
                    connection?.catalog_service_slug ??
                    connection?.slug ??
                    item.service_id
                  }
                  iconUrl={connection?.icon_url || connection?.catalog_icon_url}
                  isMutating={!isConnected || mutatingIds.has(item.id)}
                  onRevoke={() => handleRevoke(item)}
                />
              );
            }}
            ListHeaderComponent={
              approvalsQuery.isError ? (
                <View style={styles.queryNotice}>
                  <Text style={styles.subtitle}>
                    Could not load active grants. Displayed grants may be out of
                    date.
                  </Text>
                  <PrimaryButton
                    label="Retry active grants"
                    kind="ghost"
                    disabled={!isConnected}
                    onPress={() => {
                      void approvalsQuery.refetch();
                    }}
                  />
                </View>
              ) : null
            }
            ListEmptyComponent={
              !isConnected ? (
                <View style={styles.queryNotice}>
                  <Text style={styles.subtitle}>
                    Connect to check active grants.
                  </Text>
                </View>
              ) : approvalsQuery.isPending ? (
                <ActivityIndicator
                  accessibilityLabel="Loading active grants"
                  color={colors.primary}
                  style={styles.loadingFooter}
                />
              ) : !approvalsQuery.isError ? (
                <View style={styles.queryNotice}>
                  <Text style={styles.emptyTitle}>No active grants</Text>
                  <Text style={styles.subtitle}>
                    Reusable grants appear after you approve a grant-mode
                    request.
                  </Text>
                </View>
              ) : null
            }
            contentContainerStyle={styles.listContent}
            ItemSeparatorComponent={() => <View style={styles.separator} />}
            onEndReached={() => {
              if (
                approvalsQuery.hasNextPage &&
                !approvalsQuery.isFetchingNextPage
              )
                void approvalsQuery.fetchNextPage();
            }}
            onEndReachedThreshold={0.5}
            ListFooterComponent={
              approvalsQuery.isFetchingNextPage ? (
                <ActivityIndicator
                  style={styles.loadingFooter}
                  color={colors.primary}
                />
              ) : null
            }
            refreshControl={
              <RefreshControl
                refreshing={isPullRefreshing}
                onRefresh={handlePullRefresh}
                tintColor={colors.primary}
              />
            }
          />
        </>
      )}

      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    header: {
      paddingHorizontal: spacing.xxl,
      paddingTop: spacing.sm,
      minHeight: 41,
      flexDirection: "row",
      alignItems: "flex-start",
      justifyContent: "space-between",
      gap: spacing.lg,
    },
    headerCopy: {
      flex: 1,
      gap: spacing.xxs,
    },
    subtitle: {
      ...typeScale.label,
      color: c.textSecondary,
      marginBottom: spacing.md,
    },
    refreshButton: {
      width: 44,
      height: 44,
      borderRadius: radius.md,
      borderWidth: 1,
      borderColor: c.border,
      backgroundColor: c.ghostBg,
      alignItems: "center",
      justifyContent: "center",
    },
    listContent: {
      paddingHorizontal: spacing.xxl,
      paddingBottom: spacing.huge,
    },
    separator: {
      height: spacing.sm,
    },
    queryNotice: { paddingVertical: spacing.xxl, gap: spacing.sm },
    emptyTitle: { ...typeScale.bodyStrong, color: c.textPrimary },
    loadingFooter: {
      paddingVertical: spacing.xl,
    },
  });
