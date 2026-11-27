import { localizedToast } from "../../i18n/localizedToast";
import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { ServiceIcon } from "../../components/ServiceIcon";
import { getFrontendUrl } from "../../lib/env";
import { useCallback, useEffect, useRef, useState } from "react";
import { useWorkspaceBack } from "../../app/WorkspaceHeader";
import { capture } from "../../lib/telemetry";
import { AppState, Linking, StyleSheet, Text, View } from "react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useFocusEffect } from "@react-navigation/native";
import {
  onlineManager,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { SkeletonRows } from "../../components/Skeleton";
import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { setActiveChallengeId } from "../../lib/notifications/pushNotifications";
import { ApprovalDecisionPair } from "../../components/ChallengeCard";
import {
  SettingsGroup,
  SettingsPage,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { ApprovalEvidence, approvalExpiryLabel } from "./ApprovalEvidence";
import { mobileApi } from "../../lib/api/mobileApi";
import {
  getChallengeActionState,
  getChallengeQueryErrorMessage,
  getDecisionErrorMessage,
  formatGrantDuration,
} from "./challengeUiState";
import { useTheme } from "../../theme/ThemeContext";
import { spacing, typeScale } from "../../theme/designTokens";
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

export function ActivityDetailScreen({ navigation, route }: Props) {
  useAppLanguage();
  const { colors } = useTheme();
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
  decidable.current = data ? getChallengeActionState(data).canDecide : false;
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
      if (data?.decision_destination === "originating_owner_chat")
        throw new Error(translateApp("Review this request in your original private chat with NyxBot."));
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
      setToast(localizedToast(() => ({
        message: decision === "APPROVE" ? translateApp("Approved")
            : translateApp("Denied"),
        kind: "success",
      })));
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
      setToast(localizedToast(() => ({ message: getDecisionErrorMessage(err), kind: "error" })));
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
      <SettingsPage>
        <SkeletonRows rows={4} icon={false} accessibilityLabel={translateApp("Loading request")}
        />
        <SkeletonRows rows={2} icon={false} detail={false} />
      </SettingsPage>
    );
  }

  if (isError || !data) {
    return (
      <SettingsPage>
        <SettingsGroup
          inset={false}
          footer={getChallengeQueryErrorMessage(error)}
        >
          <SettingsRow label={translateApp("Request unavailable")} />
        </SettingsGroup>
        <SettingsGroup inset={false}>
          <SettingsRow tone="action" label={translateApp("Retry")}
            onPress={() => refetch()} />
        </SettingsGroup>
      </SettingsPage>
    );
  }

  const actionState = getChallengeActionState(data);
  const grantDurationLabel = formatGrantDuration(
    settingsQuery.data?.grant_expiry_days,
  );
  const isGrantMode = data.approval_mode === "grant";
  const summary =
    data.summary.trim() || `${data.action} ${data.resource}`.trim();
  const status =
    actionState.statusLabel;
  const risk = translateApp(
    data.risk_level.charAt(0).toUpperCase() + data.risk_level.slice(1),
  );
  const decisionNotes = [
    data.is_destructive === true ? translateApp("This operation is destructive.")
      : null,
    actionState.reason,
    actionState.canDecide
      ? isGrantMode
        ? translateApp(
            "Approving allows matching requests for {{label}} without another decision.",
            { label: grantDurationLabel },
          )
        : translateApp("Approval applies only to this request.")
      : null,
  ].filter(Boolean);

  return (
    <View style={styles.fill}>
      <SettingsPage>
        <SettingsGroup
          footer={
            data.from_org_policy
              ? translateApp("On behalf of {{organization}}.", {
                  organization: data.org_name ?? "your org",
                })
              : undefined
          }
        >
          <SettingsRow
            leading={<ServiceIcon slug={data.service_slug} size="md" />}
            label={data.title}
            detail={summary}
          />
        </SettingsGroup>
        <SettingsGroup title={translateApp("Request")} inset={false}>
          <SettingsRow label={translateApp("Action")} value={data.action} />
          <SettingsRow label={translateApp("Resource")} value={data.resource} />
          <SettingsRow label={translateApp("Client")}
            value={data.request_context.client} />
          <SettingsRow
            label={translateApp("Requester")}
            value={data.request_context.requester}
          />
          {data.from_org_policy ? (
            <SettingsRow label={translateApp("Org")}
              value={data.org_name ?? translateApp("Unnamed org")} />
          ) : null}
        </SettingsGroup>
        <SettingsGroup
          title={translateApp("Decision")}
          inset={false}
          footer={
            decisionNotes.length ? (
              <Text
                style={[
                  styles.footer,
                  {
                    color:
                      data.is_destructive === true
                        ? colors.danger
                        : colors.textMuted,
                  },
                ]}
              >
                {decisionNotes.join(" ")}
              </Text>
            ) : undefined
          }
        >
          <SettingsRow
            label={translateApp("Status")}
            value={status}
            accessibilityLabel={translateApp("Status, {{status}}", {
              status: status,
            })}
          />
          <SettingsRow
            label={translateApp("Risk level")}
            value={risk}
            valueTone={data.risk_level === "high" ? "danger" : "default"}
          />
          <SettingsRow
            label={translateApp("Approval")}
            value={translateApp(isGrantMode ? "Grant mode" : "Per-request")}
          />
          {isGrantMode && (
            <SettingsRow label={translateApp("Grant duration")}
              value={grantDurationLabel} />
          )}
          <SettingsRow label={translateApp("Expires")}
            value={approvalExpiryLabel(data)} />
        </SettingsGroup>
        <ApprovalEvidence challenge={data} />
        {data.execution_proposal && data.decision_destination !== "originating_owner_chat" && (
          <SettingsGroup inset={false}>
            <SettingsRow
              tone="action"
              label={translateApp("Review in the web app")}
              accessibilityRole="link"
              onPress={() =>
                void Linking.openURL(`${getFrontendUrl()}/assistant/approvals`)
              }
            />
          </SettingsGroup>
        )}
        {actionState.canDecide && (
          <SettingsGroup inset={false}>
            <ApprovalDecisionPair
              divider={false}
              disabled={decideMutation.isPending}
              onApprove={() => decideMutation.mutate("APPROVE")}
              onDeny={() => decideMutation.mutate("DENY")}
            />
          </SettingsGroup>
        )}
      </SettingsPage>
      <ToastOverlay toast={toast} bottom={64} />
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  footer: { ...typeScale.small, paddingHorizontal: spacing.xxl },
});
