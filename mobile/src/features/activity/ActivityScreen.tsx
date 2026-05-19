import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { capture } from "../../lib/telemetry";
import {
  ActivityIndicator,
  Alert,
  Dimensions,
  FlatList,
  Modal,
  Pressable,
  RefreshControl,
  ScrollView,
  SectionList,
  StyleSheet,
  Text,
  View,
} from "react-native";
import Animated, {
  useSharedValue,
  useAnimatedStyle,
  withTiming,
  runOnJS,
} from "react-native-reanimated";
import {
  GestureHandlerRootView,
  Gesture,
  GestureDetector,
} from "react-native-gesture-handler";
import { useNavigation, useRoute, type RouteProp } from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ScreenContainer } from "../../components/ScreenContainer";

import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { SegmentControl } from "../../components/SegmentControl";
import { ChallengeCard } from "../../components/ChallengeCard";
import { GrantCard } from "../../components/GrantCard";
import { HistoryCard, HistorySectionHeader } from "../../components/HistoryCard";
import { EmptyState } from "../../components/EmptyState";
import { OfflineBanner } from "../../components/OfflineBanner";
import { FullScreenLoading } from "../../components/FullScreenLoading";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { mobileApi } from "../../lib/api/mobileApi";
import { createIdempotencyKey } from "../../lib/api/idempotency";
import { getDecisionErrorMessage, formatGrantDuration, getChallengeActionState } from "./challengeUiState";
import { StatusBadge } from "../../components/StatusBadge";
import { PrimaryButton } from "../../components/PrimaryButton";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { createFlowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import type { RootStackParamList } from "../../app/AppNavigator";
import type { ActivitySegment } from "./activityTypes";
import type { ApprovalMode, ChallengeDetail, ApprovalItem } from "../../lib/api/types";
import { usePushPollingActive } from "../../lib/notifications/pushPollingSignal";

type Nav = NativeStackNavigationProp<RootStackParamList>;

function groupHistoryByDate(items: ChallengeDetail[]) {
  const today = new Date();
  const yesterday = new Date(today);
  yesterday.setDate(yesterday.getDate() - 1);

  const todayStr = today.toDateString();
  const yesterdayStr = yesterday.toDateString();

  const groups: Record<string, ChallengeDetail[]> = {};
  for (const item of items) {
    const d = new Date(item.created_at);
    let key: string;
    if (d.toDateString() === todayStr) key = "Today";
    else if (d.toDateString() === yesterdayStr) key = "Yesterday";
    else key = d.toLocaleDateString("en-US", { month: "short", day: "numeric" });

    if (!groups[key]) groups[key] = [];
    groups[key]!.push(item);
  }

  return Object.entries(groups).map(([title, data]) => ({ title, data }));
}

const SCREEN_HEIGHT = Dimensions.get("window").height;
const SHEET_TOP = 120;
const SHEET_HEIGHT = SCREEN_HEIGHT - SHEET_TOP;
const CLOSE_THRESHOLD = 60;

function DetailRow({ label, value, isLast, valueColor, flowStyles }: {
  label: string;
  value: string;
  isLast?: boolean;
  valueColor?: string;
  flowStyles: ReturnType<typeof createFlowStyles>;
}) {
  return (
    <View style={isLast ? flowStyles.rowLast : flowStyles.row}>
      <Text style={flowStyles.rowLabel}>{label}</Text>
      <Text style={[flowStyles.rowValue, valueColor ? { color: valueColor } : undefined]}>
        {value}
      </Text>
    </View>
  );
}

function ChallengeDetailSheet({
  challenge,
  grantDurationLabel,
  onClose,
  onApprove,
  onDeny,
  isMutating,
}: {
  challenge: ChallengeDetail | null;
  grantDurationLabel: string;
  onClose: () => void;
  onApprove?: (id: string) => void;
  onDeny?: (id: string) => void;
  isMutating?: boolean;
}) {
  const { colors } = useTheme();
  const sheetStyles = useMemo(() => createSheetStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const [modalVisible, setModalVisible] = useState(false);
  const isDismissing = useRef(false);
  // Keep a snapshot of the challenge so we can render during the dismiss animation
  const displayChallenge = useRef<ChallengeDetail | null>(null);
  const translateY = useSharedValue(SHEET_HEIGHT);

  if (challenge) {
    displayChallenge.current = challenge;
  }

  useEffect(() => {
    if (challenge) {
      isDismissing.current = false;
      translateY.value = SHEET_HEIGHT;
      setModalVisible(true);
      requestAnimationFrame(() => {
        translateY.value = withTiming(0, { duration: 300 });
      });
    } else if (modalVisible) {
      // Animate out, then hide the modal
      isDismissing.current = true;
      translateY.value = withTiming(SHEET_HEIGHT, { duration: 280 }, (finished) => {
        if (finished) {
          runOnJS(setModalVisible)(false);
        }
      });
    }
  }, [challenge, translateY]); // eslint-disable-line react-hooks/exhaustive-deps

  const handleClose = useCallback(() => {
    if (isDismissing.current) return;
    isDismissing.current = true;
    translateY.value = withTiming(SHEET_HEIGHT, { duration: 280 }, (finished) => {
      if (finished) {
        runOnJS(onClose)();
        runOnJS(setModalVisible)(false);
      }
    });
  }, [onClose, translateY]);

  const panGesture = useMemo(
    () =>
      Gesture.Pan()
        .onUpdate((e) => {
          "worklet";
          if (e.translationY > 0) {
            translateY.value = e.translationY;
          }
        })
        .onEnd((e) => {
          "worklet";
          if (e.translationY > CLOSE_THRESHOLD) {
            translateY.value = withTiming(SHEET_HEIGHT, { duration: 250 }, (finished) => {
              if (finished) {
                runOnJS(onClose)();
                runOnJS(setModalVisible)(false);
              }
            });
          } else {
            translateY.value = withTiming(0, { duration: 250 });
          }
        }),
    [onClose, translateY]
  );

  const sheetAnimatedStyle = useAnimatedStyle(() => ({
    transform: [{ translateY: translateY.value }],
  }));

  const backdropAnimatedStyle = useAnimatedStyle(() => ({
    opacity: 0.55 * Math.max(0, 1 - translateY.value / SHEET_HEIGHT),
  }));

  const shown = displayChallenge.current;
  if (!shown) return null;

  const actionState = getChallengeActionState(shown);
  const riskColorMap = { high: colors.riskHigh.text, medium: colors.riskMedium.text, low: colors.riskLow.text };
  const riskColor = riskColorMap[shown.risk_level];
  const isGrantMode = shown.approval_mode === "grant";

  return (
    <Modal
      visible={modalVisible}
      transparent
      animationType="none"
      statusBarTranslucent
      onRequestClose={handleClose}
    >
      <GestureHandlerRootView style={sheetStyles.modalRoot}>
        <Animated.View style={[sheetStyles.backdrop, backdropAnimatedStyle]} pointerEvents="auto">
          <Pressable style={StyleSheet.absoluteFill} onPress={handleClose} />
        </Animated.View>

        <Animated.View style={[sheetStyles.sheet, sheetAnimatedStyle]}>
          <GestureDetector gesture={panGesture}>
            <Animated.View style={sheetStyles.handleArea}>
              <View style={sheetStyles.handle} />
            </Animated.View>
          </GestureDetector>

          <View style={sheetStyles.sheetHeader}>
            <Text style={sheetStyles.sheetTitle}>Challenge Detail</Text>
            <Pressable style={sheetStyles.closeBtn} onPress={handleClose}>
              <Text style={sheetStyles.closeBtnText}>✕</Text>
            </Pressable>
          </View>

          <ScrollView style={sheetStyles.sheetBody} contentContainerStyle={sheetStyles.sheetBodyContent}>
            {shown.from_org_policy ? (
              <Text style={sheetStyles.orgContext}>
                On behalf of {shown.org_name ?? "your org"}
              </Text>
            ) : null}
            <View style={sheetStyles.detailCard}>
              <Text style={flowStyles.cardTitle}>Request Context</Text>
              <DetailRow label="Action" value={shown.action} flowStyles={flowStyles} />
              <DetailRow label="Resource" value={shown.resource} flowStyles={flowStyles} />
              <DetailRow label="Service" value={shown.title} flowStyles={flowStyles} />
              <DetailRow label="Client" value={shown.request_context.client} flowStyles={flowStyles} />
              <DetailRow label="Risk Level" value={shown.risk_level.toUpperCase()} valueColor={riskColor} flowStyles={flowStyles} />
              <DetailRow label="Status" value={actionState.statusLabel} flowStyles={flowStyles} />
              {isGrantMode && <DetailRow label="Grant Duration" value={grantDurationLabel} flowStyles={flowStyles} />}
              {shown.from_org_policy ? (
                <DetailRow
                  label="Org"
                  value={shown.org_name ?? "Unnamed org"}
                  flowStyles={flowStyles}
                />
              ) : null}
              <DetailRow label="Location" value={shown.request_context.location} isLast flowStyles={flowStyles} />
            </View>

            {actionState.reason ? (
              <View style={sheetStyles.stateNotice}>
                <Text style={sheetStyles.stateNoticeText}>{actionState.reason}</Text>
              </View>
            ) : null}

            {actionState.canDecide && onApprove && onDeny && (
              <View style={flowStyles.actionWrap}>
                <PrimaryButton
                  label="Approve"
                  onPress={() => onApprove(shown.id)}
                  disabled={isMutating}
                />
                <PrimaryButton
                  label="Deny"
                  kind="danger"
                  onPress={() => onDeny(shown.id)}
                  disabled={isMutating}
                />
              </View>
            )}
          </ScrollView>
        </Animated.View>
      </GestureHandlerRootView>
    </Modal>
  );
}

export function ActivityScreen() {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const navigation = useNavigation<Nav>();
  const route = useRoute<RouteProp<RootStackParamList, "Activity">>();
  const queryClient = useQueryClient();
  const { isConnected, recheckConnection } = useNetworkStatus();
  const isPolling = usePushPollingActive();
  const [activeSegment, setActiveSegment] = useState<ActivitySegment>("pending");
  const [toast, setToast] = useState<ToastState | null>(null);
  const [mutatingIds, setMutatingIds] = useState<Set<string>>(new Set());
  const [detailChallenge, setDetailChallenge] = useState<ChallengeDetail | null>(null);
  // Track when the approval-detail sheet was opened so we can report
  // both the view duration on abandonment and the view->tap latency on
  // decision emission.
  const detailOpenedAtRef = useRef<number | null>(null);
  const detailDecidedRef = useRef<boolean>(false);
  // Track when the pending-list tab first surfaced approvals in this
  // view session. Used as the decision-latency fallback for INLINE
  // approve/deny taps on a list card (no detail sheet opened). Without
  // this, inline decisions reported `decision_ms = 0` and polluted the
  // latency metric with synthetic zeros.
  const listPendingShownAtRef = useRef<number | null>(null);
  // Tracks whether the currently-open detail sheet was opened from a
  // path where a decision is actually possible (i.e. PENDING status).
  // History-list taps open the same sheet in read-only mode, so
  // closing THAT sheet isn't an "abandonment" -- the user was just
  // reviewing, never had a decision to make. This ref lets
  // closeApprovalDetail gate `ui.mobile_dialog_abandoned` accordingly.
  const detailDecidableRef = useRef<boolean>(false);

  // Open the approval-detail sheet and emit the pair of events the
  // telemetry spec wants: `mobile.approval_viewed` (device-side) and
  // `ui.mobile_dialog_opened` (CTA taxonomy). Centralizing the opener
  // guarantees both sites (card tap + deep-link) record consistently.
  const openApprovalDetail = useCallback(
    (challenge: ChallengeDetail, entryPoint: string) => {
      detailOpenedAtRef.current = Date.now();
      detailDecidedRef.current = false;
      detailDecidableRef.current = challenge.status === "PENDING";
      capture({
        name: "mobile.approval_viewed",
        props: {
          // Use the backend's stable slug (catalog slug or
          // `UserService.slug`), NOT the display title, so the funnel
          // groups by the underlying service rather than user-editable
          // text and custom services don't fragment by renamed titles.
          service_slug: challenge.service_slug || "unknown",
          mode: challenge.approval_mode,
        },
      });
      capture({
        name: "ui.mobile_dialog_opened",
        props: { dialog_id: "approval_detail", entry_point: entryPoint },
      });
      setDetailChallenge(challenge);
    },
    []
  );

  const closeApprovalDetail = useCallback(() => {
    const openedAt = detailOpenedAtRef.current;
    // Only count closing the sheet as "abandonment" when the user was
    // looking at a PENDING approval (decision was possible). History
    // or already-decided items open the same sheet in read-only mode;
    // closing them is normal review behavior, not abandonment.
    if (
      openedAt != null
      && !detailDecidedRef.current
      && detailDecidableRef.current
    ) {
      capture({
        name: "ui.mobile_dialog_abandoned",
        props: {
          dialog_id: "approval_detail",
          final_step: 1,
          duration_ms: Math.max(0, Date.now() - openedAt),
        },
      });
    }
    detailOpenedAtRef.current = null;
    detailDecidedRef.current = false;
    detailDecidableRef.current = false;
    setDetailChallenge(null);
  }, []);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(t);
  }, [toast]);

  // --- Queries ---
  const PAGE_SIZE = 20;

  const getNextPageParam = (lastPage: { total: number; page: number; per_page: number }) => {
    const totalPages = Math.ceil(lastPage.total / lastPage.per_page);
    return lastPage.page < totalPages ? lastPage.page + 1 : undefined;
  };

  const pendingQuery = useInfiniteQuery({
    queryKey: ["challenges", "pending"],
    queryFn: ({ pageParam }) => mobileApi.getChallenges(pageParam, PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam,
    refetchInterval: isPolling ? 30_000 : false,
  });

  const approvalsQuery = useInfiniteQuery({
    queryKey: ["approvals"],
    queryFn: ({ pageParam }) => mobileApi.getApprovals(pageParam, PAGE_SIZE),
    initialPageParam: 1,
    getNextPageParam,
    refetchInterval: isPolling ? 30_000 : false,
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
    refetchInterval: isPolling ? 30_000 : false,
  });

  const pendingItems = pendingQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const activeItems = approvalsQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const historyItems = historyQuery.data?.pages.flatMap((p) => p.items) ?? [];
  const grantDurationLabel = formatGrantDuration(settingsQuery.data?.grant_expiry_days);

  const pendingCount = pendingItems.length;
  const activeCount = activeItems.length;

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

  // --- Deep-link / push-notification: auto-open sheet for a specific challenge ---
  const deepLinkChallengeId = route.params?.challengeId;
  const deepLinkConsumedRef = useRef<string | null>(null);

  useEffect(() => {
    if (!deepLinkChallengeId || deepLinkChallengeId === deepLinkConsumedRef.current) return;
    deepLinkConsumedRef.current = deepLinkChallengeId;
    navigation.setParams({ challengeId: undefined });

    const found = pendingItems.find((c) => c.id === deepLinkChallengeId);
    if (found) {
      openApprovalDetail(found, "deep_link");
      return;
    }

    // Not in local cache yet — fetch directly
    mobileApi.getChallengeById(deepLinkChallengeId).then((challenge) => {
      openApprovalDetail(challenge, "deep_link");
    }).catch(() => {
      setToast({ message: "Challenge not found", kind: "error" });
    });
  }, [deepLinkChallengeId, pendingItems, navigation, openApprovalDetail]);

  // --- Mutations ---
  const decideMutation = useMutation({
    mutationFn: async ({ id, decision }: { id: string; decision: "APPROVE" | "DENY"; approvalMode: ApprovalMode }) => {
      const durationSec = decision === "APPROVE" ? (settingsQuery.data?.grant_expiry_days ?? 30) * 86400 : undefined;
      const idempotencyKey = createIdempotencyKey("decision", id);
      return mobileApi.submitDecision(id, decision, durationSec);
    },
    onMutate: ({ id }) => {
      // Snapshot view->tap latency HERE (so offline/expired failures
      // don't see a delayed clock), but DEFER the actual
      // `ui.mobile_decision_made` emit until onSuccess. Emitting in
      // onMutate would overcount on failures (offline, session
      // expired, challenge already decided elsewhere). Mark the detail
      // flow as "decided" so closeApprovalDetail doesn't emit a
      // dialog_abandoned for the in-flight attempt.
      //
      // Latency reference precedence:
      //   1. detail sheet open time (user reviewed the full approval)
      //   2. pending-list shown time (inline decision from card tap)
      //   3. tap-time (fallback: decision_ms = 0 only when neither ref
      //      is set -- rare: e.g. deep-link that bypasses both)
      const openedAt = detailOpenedAtRef.current ?? listPendingShownAtRef.current;
      const decisionMs = openedAt != null ? Math.max(0, Date.now() - openedAt) : 0;
      detailDecidedRef.current = true;
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
      detailOpenedAtRef.current = null;
      detailDecidedRef.current = false;
      setDetailChallenge(null);

      const isGrant = decision === "APPROVE" && approvalMode === "grant";
      const targetSegment: ActivitySegment = isGrant ? "active" : "history";
      const targetLabel = isGrant ? "View in Active" : "View in History";

      setToast({
        message: decision === "APPROVE" ? "Approved" : "Denied",
        kind: "success",
        action: { label: targetLabel, onPress: () => setActiveSegment(targetSegment) },
      });
    },
    onError: (error, { id }) => {
      // Decision didn't stick -- clear the "decided" flag so a subsequent
      // sheet close is counted as an abandonment. Intentionally KEEP
      // `detailOpenedAtRef` so a retry from the same sheet still measures
      // `decision_ms` from the original open, and `closeApprovalDetail()`
      // can still emit `ui.mobile_dialog_abandoned` with the real
      // duration. The ref clears naturally on successful decide or sheet
      // close.
      detailDecidedRef.current = false;
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

  const handleRevoke = useCallback((grant: ApprovalItem) => {
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
          revokeMutation.mutate({ id: grant.id, orgId: grant.org_id ?? null });
        },
      },
    ]);
  }, [revokeMutation]);

  const handleRefresh = useCallback(() => {
    if (activeSegment === "pending") void pendingQuery.refetch();
    if (activeSegment === "active") void approvalsQuery.refetch();
    if (activeSegment === "history") void historyQuery.refetch();
  }, [activeSegment, pendingQuery, approvalsQuery, historyQuery]);

  const handleOfflineRetry = useCallback(async () => {
    const online = await recheckConnection();
    if (online) {
      handleRefresh();
    } else {
      setToast({ message: "Still offline — will retry when connected", kind: "error" });
    }
  }, [recheckConnection, handleRefresh]);

  const isRefreshing =
    (activeSegment === "pending" && pendingQuery.isRefetching && !pendingQuery.isFetchingNextPage) ||
    (activeSegment === "active" && approvalsQuery.isRefetching && !approvalsQuery.isFetchingNextPage) ||
    (activeSegment === "history" && historyQuery.isRefetching && !historyQuery.isFetchingNextPage);

  // --- Loading states ---
  const isInitialLoading =
    pendingQuery.isLoading && approvalsQuery.isLoading;

  if (isInitialLoading) {
    return <FullScreenLoading title="Loading activity..." subtitle="Fetching your challenges and grants" />;
  }

  // --- Sorted active items (urgent first) ---
  const sortedActiveItems = [...activeItems].sort(
    (a, b) => new Date(a.expires_at).getTime() - new Date(b.expires_at).getTime()
  );

  const historySections = groupHistoryByDate(
    [...historyItems].sort((a, b) => new Date(b.created_at).getTime() - new Date(a.created_at).getTime())
  );

  const segments = [
    { label: "Pending", count: pendingCount },
    { label: "Active", count: activeCount },
    { label: "History" },
  ];

  const segmentIndex = activeSegment === "pending" ? 0 : activeSegment === "active" ? 1 : 2;

  return (
    <ScreenContainer>
      <View style={styles.header}>
        <Text style={styles.title}>Activity</Text>
        <Text style={styles.subtitle}>
          {pendingCount} pending · {activeCount} active grant{activeCount !== 1 ? "s" : ""}
        </Text>
      </View>

      <View style={styles.segmentWrap}>
        {!isConnected && <OfflineBanner onRetry={handleOfflineRetry} />}
        <SegmentControl
          segments={segments}
          activeIndex={segmentIndex}
          onPress={(i) => {
            const seg: ActivitySegment[] = ["pending", "active", "history"];
            const next = seg[i] ?? "pending";
            if (next !== activeSegment) {
              const resultCount =
                next === "pending"
                  ? pendingCount
                  : next === "active"
                    ? activeCount
                    : historyItems.length;
              capture({
                name: "ui.mobile_list_filtered",
                props: {
                  list: "activity",
                  filter: next,
                  result_count: resultCount,
                },
              });
            }
            setActiveSegment(next);
          }}
        />
      </View>

      {activeSegment === "pending" && (
        pendingItems.length === 0 ? (
          <View style={styles.emptyWrap}>
            <EmptyState preset="pendingEmpty" />
          </View>
        ) : (
          <FlatList
            data={pendingItems}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <ChallengeCard
                challenge={item}
                grantDurationLabel={grantDurationLabel}
                isMutating={mutatingIds.has(item.id)}
                onPress={() => openApprovalDetail(item, "pending_list")}
                onApprove={() => decideMutation.mutate({ id: item.id, decision: "APPROVE", approvalMode: item.approval_mode })}
                onDeny={() => decideMutation.mutate({ id: item.id, decision: "DENY", approvalMode: item.approval_mode })}
              />
            )}
            contentContainerStyle={styles.listContent}
            ItemSeparatorComponent={() => <View style={styles.separator} />}
            onEndReached={() => {
              if (pendingQuery.hasNextPage && !pendingQuery.isFetchingNextPage) pendingQuery.fetchNextPage();
            }}
            onEndReachedThreshold={0.5}
            ListFooterComponent={pendingQuery.isFetchingNextPage ? <ActivityIndicator style={styles.loadingFooter} color={colors.primary} /> : null}
            refreshControl={
              <RefreshControl refreshing={isRefreshing} onRefresh={handleRefresh} tintColor={colors.primary} />
            }
          />
        )
      )}

      {activeSegment === "active" && (
        sortedActiveItems.length === 0 ? (
          <View style={styles.emptyWrap}>
            <EmptyState preset="activeEmpty" />
          </View>
        ) : (
          <FlatList
            data={sortedActiveItems}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <GrantCard
                grant={item}
                isMutating={mutatingIds.has(item.id)}
                onRevoke={() => handleRevoke(item)}
              />
            )}
            contentContainerStyle={styles.listContent}
            ItemSeparatorComponent={() => <View style={styles.separator} />}
            onEndReached={() => {
              if (approvalsQuery.hasNextPage && !approvalsQuery.isFetchingNextPage) approvalsQuery.fetchNextPage();
            }}
            onEndReachedThreshold={0.5}
            ListFooterComponent={approvalsQuery.isFetchingNextPage ? <ActivityIndicator style={styles.loadingFooter} color={colors.primary} /> : null}
            refreshControl={
              <RefreshControl refreshing={isRefreshing} onRefresh={handleRefresh} tintColor={colors.primary} />
            }
          />
        )
      )}

      {activeSegment === "history" && (
        historyItems.length === 0 ? (
          <View style={styles.emptyWrap}>
            <EmptyState preset="historyEmpty" />
          </View>
        ) : (
          <SectionList
            sections={historySections}
            keyExtractor={(item) => item.id}
            renderItem={({ item }) => (
              <HistoryCard
                item={item}
                onPress={() => openApprovalDetail(item, "history_list")}
              />
            )}
            renderSectionHeader={({ section }) => <HistorySectionHeader title={section.title} />}
            stickySectionHeadersEnabled
            contentContainerStyle={styles.listContent}
            ItemSeparatorComponent={() => <View style={styles.separator} />}
            SectionSeparatorComponent={() => <View style={styles.sectionSep} />}
            onEndReached={() => {
              if (historyQuery.hasNextPage && !historyQuery.isFetchingNextPage) historyQuery.fetchNextPage();
            }}
            onEndReachedThreshold={0.5}
            ListFooterComponent={historyQuery.isFetchingNextPage ? <ActivityIndicator style={styles.loadingFooter} color={colors.primary} /> : null}
            refreshControl={
              <RefreshControl refreshing={isRefreshing} onRefresh={handleRefresh} tintColor={colors.primary} />
            }
          />
        )
      )}

      <ChallengeDetailSheet
        challenge={detailChallenge}
        grantDurationLabel={grantDurationLabel}
        onClose={closeApprovalDetail}
        onApprove={(id) => decideMutation.mutate({ id, decision: "APPROVE", approvalMode: detailChallenge!.approval_mode })}
        onDeny={(id) => decideMutation.mutate({ id, decision: "DENY", approvalMode: detailChallenge!.approval_mode })}
        isMutating={mutatingIds.has(detailChallenge?.id ?? "")}
      />
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) => StyleSheet.create({
  header: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.sm,
    minHeight: 41,
    gap: spacing.xxs,
  },
  // DESIGN.md §PageHeader: mobile page title is text-[22px] font-bold leading-none
  // tracking-tight with -0.03em letter-spacing. Mobile downshift is intentional.
  title: {
    ...typeScale.pageHeader,
    color: c.textPrimary,
  },
  subtitle: {
    ...typeScale.label,
    color: c.textSecondary,
    marginBottom: spacing.md,
  },
  segmentWrap: {
    paddingHorizontal: spacing.xxl,
  },
  listContent: {
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.huge,
  },
  separator: {
    height: spacing.sm,
  },
  sectionSep: {
    height: spacing.xs,
  },
  emptyWrap: {
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.xxl,
  },
  loadingFooter: {
    paddingVertical: spacing.xl,
  },
});

const createSheetStyles = (c: ThemeColors) => StyleSheet.create({
  modalRoot: {
    flex: 1,
  },
  backdrop: {
    ...StyleSheet.absoluteFillObject,
    backgroundColor: c.overlayBg,
  },
  sheet: {
    position: "absolute",
    top: SHEET_TOP,
    left: 0,
    right: 0,
    bottom: 0,
    backgroundColor: c.bg,
    // Bottom sheets use a larger top radius than card lg (10) per
    // iOS HIG; keep 24 explicitly as a sheet-only override.
    borderTopLeftRadius: 24,
    borderTopRightRadius: 24,
    borderWidth: 1,
    borderBottomWidth: 0,
    borderColor: c.border,
    shadowColor: c.shadowColor,
    shadowOffset: { width: 0, height: -10 },
    shadowOpacity: 0.4,
    shadowRadius: 40,
    elevation: 24,
    overflow: "hidden",
  },
  handleArea: {
    alignItems: "center",
    paddingTop: spacing.md,
    paddingBottom: spacing.xs + spacing.xxs,
  },
  handle: {
    width: 36,
    height: 4,
    borderRadius: radius.pill,
    backgroundColor: c.handleBg,
  },
  sheetHeader: {
    flexDirection: "row",
    alignItems: "center",
    justifyContent: "space-between",
    paddingHorizontal: spacing.xxl,
    paddingBottom: spacing.lg,
    borderBottomWidth: 1,
    borderBottomColor: c.borderSoft,
  },
  sheetTitle: {
    ...typeScale.h2,
    color: c.textPrimary,
  },
  closeBtn: {
    width: 30,
    height: 30,
    borderRadius: radius.full,
    backgroundColor: c.primaryGlow,
    borderWidth: 1,
    borderColor: c.borderSoft,
    alignItems: "center",
    justifyContent: "center",
  },
  closeBtnText: {
    ...typeScale.description,
    color: c.textMuted,
  },
  sheetBody: {
    flex: 1,
    paddingHorizontal: spacing.xxl,
    paddingTop: spacing.xl,
  },
  sheetBodyContent: {
    paddingBottom: spacing.huge,
    gap: spacing.lg,
  },
  orgContext: {
    ...typeScale.overline,
    color: c.textSecondary,
    letterSpacing: 0.6,
  },
  // DESIGN.md §Banners & callouts: rounded-xl warning callout, theme-aware tint.
  stateNotice: {
    borderRadius: radius.lg,
    backgroundColor: c.warningTone.bg,
    borderWidth: 1,
    borderColor: c.warningTone.border,
    padding: spacing.lg,
  },
  stateNoticeText: {
    ...typeScale.label,
    color: c.warningTone.text,
  },
  detailCard: {
    // Detail-sheet content card: rounded-xl, 50%-opacity chrome.
    borderRadius: radius.lg,
    borderWidth: 1,
    borderColor: c.borderSoft,
    backgroundColor: c.cardSoft,
    padding: spacing.xxl,
    gap: spacing.md,
  },
});
