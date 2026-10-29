import { ServiceIcon } from "../../components/ServiceIcon";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { capture } from "../../lib/telemetry";
import {
  AppState,
  Pressable,
  ScrollView,
  StyleSheet,
  Text,
  View,
} from "react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useFocusEffect } from "@react-navigation/native";
import {
  onlineManager,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { ScreenContainer } from "../../components/ScreenContainer";

import { FullScreenLoading } from "../../components/FullScreenLoading";
import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { setActiveChallengeId } from "../../lib/notifications/pushNotifications";
import { StatusBadge } from "../../components/StatusBadge";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ApprovalEvidence } from "./ApprovalEvidence";
import { mobileApi } from "../../lib/api/mobileApi";
import {
  getChallengeActionState,
  getChallengeQueryErrorMessage,
  getDecisionErrorMessage,
  formatGrantDuration,
} from "./challengeUiState";
import { useTheme } from "../../theme/ThemeContext";
import type { ThemeColors } from "../../theme/mobileTheme";
import { radius, spacing, typeScale } from "../../theme/designTokens";
import { createFlowStyles } from "../../theme/flowStyles";
import type { WorkspaceContentParamList } from "../../app/navigationTypes";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import {
  APPROVAL_BACKSTOP_POLL_INTERVAL_MS,
  signalApprovalStateMayHaveChanged,
  subscribeToApprovalRefreshSignals,
} from "../../lib/notifications/approvalRefreshSignal";

type Props = NativeStackScreenProps<
  WorkspaceContentParamList,
  "ActivityDetail"
>;

function DetailRow({
  label,
  value,
  isLast,
  valueColor,
  flowStyles,
}: {
  label: string;
  value: string;
  isLast?: boolean;
  valueColor?: string;
  flowStyles: ReturnType<typeof createFlowStyles>;
}) {
  return (
    <View style={isLast ? flowStyles.rowLast : flowStyles.row}>
      <Text style={flowStyles.rowLabel}>{label}</Text>
      <Text
        style={[
          flowStyles.rowValue,
          valueColor ? { color: valueColor } : undefined,
        ]}
      >
        {value}
      </Text>
    </View>
  );
}

export function ActivityDetailScreen({ navigation, route }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const { challengeId } = route.params;
  const queryClient = useQueryClient();
  const { isConnected } = useNetworkStatus();
  const [isLiveRefreshEnabled, setIsLiveRefreshEnabled] = useState(false);
  const [toast, setToast] = useState<ToastState | null>(null);
  const returnTimer = useRef<ReturnType<typeof setTimeout> | undefined>(
    undefined,
  );
  useEffect(() => () => clearTimeout(returnTimer.current), []);

  // Same gate as ActivityScreen: suppress the in-app toast for the approval
  // that is already filling the screen.
  useEffect(() => {
    setActiveChallengeId(challengeId);
    return () => setActiveChallengeId(null);
  }, [challengeId]);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(t);
  }, [toast]);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["challenge", challengeId],
    queryFn: () => mobileApi.getChallengeById(challengeId),
    refetchInterval: isLiveRefreshEnabled
      ? APPROVAL_BACKSTOP_POLL_INTERVAL_MS
      : false,
  });

  const refreshInFlightRef = useRef<Promise<void> | null>(null);
  const queuedSignalRefreshRef = useRef(false);
  const acceptsSignalRefreshRef = useRef(false);

  const returnQueuedSignalToBus = useCallback(() => {
    if (!queuedSignalRefreshRef.current) return;
    queuedSignalRefreshRef.current = false;
    signalApprovalStateMayHaveChanged();
  }, []);

  const refreshDetailFromSignal = useCallback((): Promise<void> => {
    if (
      !acceptsSignalRefreshRef.current ||
      !onlineManager.isOnline() ||
      AppState.currentState !== "active"
    ) {
      // `deliver()` has already opened the throttle window, so this can retry
      // at most once per second. Focus/AppState/connectivity teardown removes
      // the last subscriber and cancels the timer while preserving the hint.
      signalApprovalStateMayHaveChanged();
      return Promise.resolve();
    }

    if (refreshInFlightRef.current) {
      queuedSignalRefreshRef.current = true;
      return refreshInFlightRef.current;
    }

    const refreshPromise = (async () => {
      let shouldRunAgain = false;
      try {
        do {
          queuedSignalRefreshRef.current = false;
          await refetch({ cancelRefetch: false });
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
        } while (shouldRunAgain);
      } finally {
        returnQueuedSignalToBus();
      }
    })().finally(() => {
      refreshInFlightRef.current = null;
    });

    refreshInFlightRef.current = refreshPromise;
    return refreshPromise;
  }, [refetch, returnQueuedSignalToBus]);

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
            void refreshDetailFromSignal();
          });

          const detailWasInvalidated = queryClient.getQueryState([
            "challenge",
            challengeId,
          ])?.isInvalidated;
          if (detailWasInvalidated) {
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
    }, [
      challengeId,
      isConnected,
      queryClient,
      refreshDetailFromSignal,
      returnQueuedSignalToBus,
    ]),
  );

  // view->tap latency for `ui.mobile_decision_made`; starts when the
  // approval data first resolves on this screen.
  const viewedAtRef = useRef<number | null>(null);
  const viewedEmittedRef = useRef<string | null>(null);
  const decided = useRef(false);
  const decidable = useRef(false);
  decidable.current = data?.status === "PENDING";
  useEffect(
    () => () => {
      if (viewedAtRef.current != null && decidable.current && !decided.current)
        capture({
          name: "ui.mobile_dialog_abandoned",
          props: {
            dialog_id: "approval_detail",
            final_step: 1,
            duration_ms: Math.max(0, Date.now() - viewedAtRef.current),
          },
        });
    },
    [],
  );
  useEffect(() => {
    if (!data) return;
    if (viewedEmittedRef.current === data.id) return;
    viewedEmittedRef.current = data.id;
    viewedAtRef.current = Date.now();
    capture({
      name: "ui.mobile_dialog_opened",
      props: {
        dialog_id: "approval_detail",
        entry_point: route.params.entryPoint ?? "deep_link",
      },
    });
    capture({
      name: "mobile.approval_viewed",
      props: {
        // Stable backend slug, not the user-editable display title.
        service_slug: data.service_slug || "unknown",
        mode: data.approval_mode,
      },
    });
  }, [data, route.params.entryPoint]);

  const settingsQuery = useQuery({
    queryKey: ["notifications", "settings"],
    queryFn: mobileApi.getNotificationSettings,
  });

  const decideMutation = useMutation({
    mutationFn: async (decision: "APPROVE" | "DENY") => {
      const durationSec =
        decision === "APPROVE"
          ? (settingsQuery.data?.grant_expiry_days ?? 30) * 86400
          : undefined;
      return mobileApi.submitDecision(challengeId, decision, durationSec);
    },
    onMutate: () => {
      // Snapshot view->tap latency at tap-time (not at server-response
      // time) but DEFER the emit to onSuccess so failures don't
      // overcount decisions.
      const openedAt = viewedAtRef.current;
      const decisionMs =
        openedAt != null ? Math.max(0, Date.now() - openedAt) : 0;
      return { decisionMs };
    },
    onSuccess: (_, decision, context) => {
      decided.current = true;
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
      void queryClient.invalidateQueries({
        queryKey: ["challenge", challengeId],
      });
      setToast({
        message: decision === "APPROVE" ? "Approved" : "Denied",
        kind: "success",
      });
      returnTimer.current = setTimeout(() => {
        if (!navigation.isFocused()) return;
        if (navigation.canGoBack()) {
          navigation.goBack();
        } else {
          navigation.replace("Activity");
        }
      }, 600);
    },
    onError: (err) => {
      setToast({ message: getDecisionErrorMessage(err), kind: "error" });
    },
  });

  useWorkspaceBack({
    active: false,
    defaultRole: "back",
    disabled: decideMutation.isPending,
    onBack: navigation.goBack,
  });

  if (isLoading) {
    return (
      <FullScreenLoading
        title="Loading detail..."
        subtitle="Fetching challenge information"
      />
    );
  }

  if (isError || !data) {
    return (
      <ScreenContainer>
        <View style={styles.errorWrap}>
          <Text style={styles.errorTitle}>Challenge Unavailable</Text>
          <Text style={styles.errorMsg}>
            {getChallengeQueryErrorMessage(error)}
          </Text>
          <View style={styles.errorActions}>
            <PrimaryButton
              label="Back"
              kind="ghost"
              onPress={() => navigation.goBack()}
            />
            <PrimaryButton label="Retry" onPress={() => refetch()} />
          </View>
        </View>
      </ScreenContainer>
    );
  }

  const actionState = getChallengeActionState(data);
  const grantDurationLabel = formatGrantDuration(
    settingsQuery.data?.grant_expiry_days,
  );
  const riskColorMap = {
    high: colors.riskHigh.text,
    medium: colors.riskMedium.text,
    low: colors.riskLow.text,
    unknown: colors.textSecondary,
  };
  const riskColor = riskColorMap[data.risk_level];
  const isGrantMode = data.approval_mode === "grant";

  return (
    <ScreenContainer>
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={[
          flowStyles.scrollContent,
          { paddingHorizontal: spacing.xxl },
        ]}
      >
        {isGrantMode && (
          <Text style={styles.screenSub}>
            Review this request before creating reusable access for{" "}
            {grantDurationLabel}.
          </Text>
        )}
        {data.from_org_policy ? (
          <Text style={styles.orgContext}>
            On behalf of {data.org_name ?? "your org"}
          </Text>
        ) : null}

        <View style={styles.detailCard}>
          <View style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
            <ServiceIcon slug={data.service_slug} size="md" />
            <Text style={flowStyles.cardTitle}>Request Context</Text>
          </View>
          <DetailRow
            label="Action"
            value={data.action}
            flowStyles={flowStyles}
          />
          <DetailRow
            label="Resource"
            value={data.resource}
            flowStyles={flowStyles}
          />
          <DetailRow
            label="Client"
            value={data.request_context.client}
            flowStyles={flowStyles}
          />
          <DetailRow
            label="Risk Level"
            value={
              data.risk_level.charAt(0).toUpperCase() + data.risk_level.slice(1)
            }
            valueColor={riskColor}
            flowStyles={flowStyles}
          />
          <DetailRow
            label="Status"
            value={actionState.statusLabel}
            valueColor={colors.warning}
            flowStyles={flowStyles}
          />
          {isGrantMode && (
            <DetailRow
              label="Grant Duration"
              value={grantDurationLabel}
              flowStyles={flowStyles}
            />
          )}
          {data.from_org_policy ? (
            <DetailRow
              label="Org"
              value={data.org_name ?? "Unnamed org"}
              flowStyles={flowStyles}
            />
          ) : null}
          <DetailRow
            label="Requester"
            value={data.request_context.requester}
            isLast
            flowStyles={flowStyles}
          />
          <ApprovalEvidence challenge={data} />
        </View>

        {actionState.reason ? (
          <View style={styles.stateNotice}>
            <Text style={styles.stateNoticeText}>{actionState.reason}</Text>
          </View>
        ) : null}

        {actionState.canDecide && (
          <View style={flowStyles.actionWrap}>
            <PrimaryButton
              label="Approve"
              onPress={() => decideMutation.mutate("APPROVE")}
              disabled={decideMutation.isPending}
            />
            <PrimaryButton
              label="Deny"
              kind="danger"
              onPress={() => decideMutation.mutate("DENY")}
              disabled={decideMutation.isPending}
            />
          </View>
        )}
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const createStyles = (c: ThemeColors) =>
  StyleSheet.create({
    backBtn: {
      paddingVertical: spacing.xs,
    },
    backText: {
      ...typeScale.caption,
      color: c.textMuted,
    },
    screenTitle: {
      ...typeScale.pageHeader,
      color: c.textPrimary,
    },
    screenSub: {
      ...typeScale.label,
      color: c.textSecondary,
    },
    orgContext: {
      ...typeScale.overline,
      color: c.textSecondary,
      letterSpacing: 1.5,
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
    // DESIGN.md §DetailSection: rounded-xl + 50%-opacity chrome border.
    detailCard: {
      borderRadius: radius.lg,
      borderWidth: 1,
      borderColor: c.borderSoft,
      backgroundColor: c.card,
      padding: spacing.xxl,
      gap: spacing.md,
    },
    errorWrap: {
      flex: 1,
      justifyContent: "center",
      alignItems: "center",
      paddingHorizontal: spacing.xxl,
      gap: spacing.lg,
    },
    errorTitle: {
      ...typeScale.h2,
      color: c.textPrimary,
    },
    errorMsg: {
      ...typeScale.description,
      color: c.textSecondary,
      textAlign: "center",
    },
    errorActions: {
      flexDirection: "row",
      gap: spacing.md,
    },
  });
