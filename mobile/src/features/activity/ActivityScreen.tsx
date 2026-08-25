import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { capture } from "../../lib/telemetry";
import {
  ActivityIndicator,
  Alert,
  AppState,
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
  cancelAnimation,
  Easing,
  withTiming,
  withRepeat,
  runOnJS,
} from "react-native-reanimated";
import {
  GestureHandlerRootView,
  Gesture,
  GestureDetector,
} from "react-native-gesture-handler";
import { useFocusEffect, useNavigation, useRoute, type RouteProp } from "@react-navigation/native";
import type { NativeStackNavigationProp } from "@react-navigation/native-stack";
import { onlineManager, useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ScreenContainer } from "../../components/ScreenContainer";

import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { setActiveChallengeId } from "../../lib/notifications/pushNotifications";
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
import {
  APPROVAL_BACKSTOP_POLL_INTERVAL_MS,
  signalApprovalStateMayHaveChanged,
  subscribeToApprovalRefreshSignals,
} from "../../lib/notifications/approvalRefreshSignal";
import { RefreshCw, ScanQrCode } from "lucide-react-native";

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
    // Query refreshes may replace the challenge object without changing which
    // sheet is open. Only an id/open-state change should replay sheet motion.
  }, [challenge?.id, translateY]); // eslint-disable-line react-hooks/exhaustive-deps

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
  const [isLiveRefreshEnabled, setIsLiveRefreshEnabled] = useState(false);
  const [activeSegment, setActiveSegment] = useState<ActivitySegment>("pending");
  const activeSegmentRef = useRef(activeSegment);
  activeSegmentRef.current = activeSegment;
  const [isPullRefreshing, setIsPullRefreshing] = useState(false);
  const [isRefreshRunning, setIsRefreshRunning] = useState(false);
  const [isUserRefreshRunning, setIsUserRefreshRunning] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const [mutatingIds, setMutatingIds] = useState<Set<string>>(new Set());
  const [detailChallenge, setDetailChallenge] = useState<ChallengeDetail | null>(null);
  const detailChallengeIdRef = useRef(detailChallenge?.id ?? null);
  detailChallengeIdRef.current = detailChallenge?.id ?? null;

  // Tell the push layer which approval is on screen, so a push for THIS
  // challenge does not raise an in-app toast over the sheet the user is
  // already acting on. Cleared on unmount so an unmounted screen never
  // keeps suppressing.
  useEffect(() => {
    setActiveChallengeId(detailChallenge?.id ?? null);
    return () => setActiveChallengeId(null);
  }, [detailChallenge?.id]);
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

  const detailChallengeId = detailChallenge?.id ?? "";
  const detailChallengeQuery = useQuery({
    queryKey: ["challenge", detailChallengeId],
    queryFn: () => mobileApi.getChallengeById(detailChallengeId),
    enabled: detailChallengeId.length > 0,
    // The tapped row keeps opening instant even before this detail key exists.
    initialData: detailChallenge ?? undefined,
    refetchInterval:
      isLiveRefreshEnabled && detailChallengeId.length > 0
        ? APPROVAL_BACKSTOP_POLL_INTERVAL_MS
        : false,
  });

  // The selected row remains the sheet's open/closed authority. Query data can
  // update that selection in place, but a late response after local close can
  // only populate cache and therefore cannot reopen the sheet.
  const sheetChallenge =
    detailChallenge && detailChallengeQuery.data?.id === detailChallenge.id
      ? detailChallengeQuery.data
      : detailChallenge;

  useEffect(() => {
    if (sheetChallenge && sheetChallenge.status !== "PENDING") {
      // A remote decision/expiry removes the actions immediately and makes a
      // later close ordinary review, not an abandoned approval decision.
      detailDecidableRef.current = false;
    }
  }, [sheetChallenge?.id, sheetChallenge?.status]);

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

  const refreshVisibleSegmentAndCounts = useCallback(async (refreshAllSegments = false) => {
    // Pending and Active appear in the header and segment badges regardless
    // of the visible list. Refreshing both keeps those counts coherent after
    // a decision moves an item between states; History only costs a request
    // when it is the list the user is actually looking at. Push signals pass
    // `refreshAllSegments` because decisions and expiries also change History,
    // which otherwise stays mounted-but-stale when the user changes segments.
    const refetches: Promise<unknown>[] = [
      pendingQuery.refetch({ cancelRefetch: false }),
      approvalsQuery.refetch({ cancelRefetch: false }),
    ];
    if (refreshAllSegments || activeSegmentRef.current === "history") {
      refetches.push(historyQuery.refetch({ cancelRefetch: false }));
    }
    if (detailChallengeIdRef.current) {
      // TanStack owns detail errors and request deduplication. The selected row
      // remains available as last-known sheet content if this refetch fails.
      refetches.push(detailChallengeQuery.refetch({ cancelRefetch: false }));
    }
    await Promise.all(refetches);
  }, [
    pendingQuery.refetch,
    approvalsQuery.refetch,
    historyQuery.refetch,
    detailChallengeQuery.refetch,
  ]);

  const handleRefresh = useCallback(
    (source: "user" | "signal" = "user"): Promise<void> => {
      if (
        !onlineManager.isOnline() ||
        AppState.currentState !== "active"
      ) {
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
    [refreshVisibleSegmentAndCounts, returnQueuedSignalToBus]
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
          ].some((queryKey) => queryClient.getQueryState(queryKey)?.isInvalidated);
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
        syncSubscription
      );

      return () => {
        stopSignalSubscription();
        appStateSubscription.remove();
      };
    }, [handleRefresh, isConnected, queryClient, returnQueuedSignalToBus])
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
      setToast({ message: "Still offline — will retry when connected", kind: "error" });
    }
  }, [recheckConnection, handleRefresh]);

  // Passive backstop polls deliberately stay invisible. Only refreshes started
  // by the user or approval signal own the header/pull refresh feedback.
  const isRefreshing = isRefreshRunning || isPullRefreshing;

  const refreshRotation = useSharedValue(0);
  useEffect(() => {
    cancelAnimation(refreshRotation);
    if (isRefreshing) {
      refreshRotation.value = 0;
      refreshRotation.value = withRepeat(
        withTiming(360, { duration: 750, easing: Easing.linear }),
        -1,
        false
      );
    } else {
      refreshRotation.value = 0;
    }

    return () => cancelAnimation(refreshRotation);
  }, [isRefreshing, refreshRotation]);

  const refreshIconAnimatedStyle = useAnimatedStyle(() => ({
    transform: [{ rotate: `${refreshRotation.value}deg` }],
  }));

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
  const isManualRefreshDisabled = !isConnected || isUserRefreshRunning || isPullRefreshing;

  return (
    <ScreenContainer>
      <View style={styles.header}>
        <View style={styles.headerCopy}>
          <Text style={styles.title}>Activity</Text>
          <Text style={styles.subtitle}>
            {pendingCount} pending · {activeCount} active grant{activeCount !== 1 ? "s" : ""}
          </Text>
        </View>
        <View style={styles.headerActions}>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Refresh activity"
            accessibilityState={{ disabled: isManualRefreshDisabled, busy: isRefreshing }}
            disabled={isManualRefreshDisabled}
            hitSlop={8}
            onPress={() => {
              void handleHeaderRefresh();
            }}
            style={({ pressed }) => [
              styles.scanLoginButton,
              pressed && styles.scanLoginButtonPressed,
            ]}
          >
            <Animated.View style={refreshIconAnimatedStyle}>
              <RefreshCw
                size={21}
                color={!isConnected ? colors.textTertiary : colors.primary}
              />
            </Animated.View>
          </Pressable>
          <Pressable
            accessibilityRole="button"
            accessibilityLabel="Scan login QR code"
            hitSlop={8}
            onPress={() => navigation.navigate("DeviceLogin", { start_scanner: true })}
            style={({ pressed }) => [
              styles.scanLoginButton,
              pressed && styles.scanLoginButtonPressed,
            ]}
          >
            <ScanQrCode size={21} color={colors.primary} />
          </Pressable>
        </View>
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
              <RefreshControl refreshing={isPullRefreshing} onRefresh={handlePullRefresh} tintColor={colors.primary} />
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
              <RefreshControl refreshing={isPullRefreshing} onRefresh={handlePullRefresh} tintColor={colors.primary} />
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
              <RefreshControl refreshing={isPullRefreshing} onRefresh={handlePullRefresh} tintColor={colors.primary} />
            }
          />
        )
      )}

      <ChallengeDetailSheet
        challenge={sheetChallenge}
        grantDurationLabel={grantDurationLabel}
        onClose={closeApprovalDetail}
        onApprove={sheetChallenge
          ? (id) => decideMutation.mutate({ id, decision: "APPROVE", approvalMode: sheetChallenge.approval_mode })
          : undefined}
        onDeny={sheetChallenge
          ? (id) => decideMutation.mutate({ id, decision: "DENY", approvalMode: sheetChallenge.approval_mode })
          : undefined}
        isMutating={mutatingIds.has(sheetChallenge?.id ?? "")}
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
    flexDirection: "row",
    alignItems: "flex-start",
    justifyContent: "space-between",
    gap: spacing.lg,
  },
  headerCopy: {
    flex: 1,
    gap: spacing.xxs,
  },
  headerActions: {
    flexDirection: "row",
    gap: spacing.sm,
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
  scanLoginButton: {
    width: 44,
    height: 44,
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.ghostBg,
    alignItems: "center",
    justifyContent: "center",
  },
  scanLoginButtonPressed: {
    opacity: 0.7,
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
