import { useEffect, useMemo, useRef, useState } from "react";
import { capture } from "../../lib/telemetry";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ScreenContainer } from "../../components/ScreenContainer";

import { FullScreenLoading } from "../../components/FullScreenLoading";
import { ToastOverlay, type ToastState } from "../../components/ToastOverlay";
import { StatusBadge } from "../../components/StatusBadge";
import { PrimaryButton } from "../../components/PrimaryButton";
import { mobileApi } from "../../lib/api/mobileApi";
import { createIdempotencyKey } from "../../lib/api/idempotency";
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
import type { RootStackParamList } from "../../app/AppNavigator";

type Props = NativeStackScreenProps<RootStackParamList, "ActivityDetail">;

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

export function ActivityDetailScreen({ navigation, route }: Props) {
  const { colors } = useTheme();
  const styles = useMemo(() => createStyles(colors), [colors]);
  const flowStyles = useMemo(() => createFlowStyles(colors), [colors]);
  const { challengeId } = route.params;
  const queryClient = useQueryClient();
  const [toast, setToast] = useState<ToastState | null>(null);

  useEffect(() => {
    if (!toast) return;
    const t = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(t);
  }, [toast]);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["challenge", challengeId],
    queryFn: () => mobileApi.getChallengeById(challengeId),
  });

  // view->tap latency for `ui.mobile_decision_made`; starts when the
  // approval data first resolves on this screen.
  const viewedAtRef = useRef<number | null>(null);
  const viewedEmittedRef = useRef<string | null>(null);
  useEffect(() => {
    if (!data) return;
    if (viewedEmittedRef.current === data.id) return;
    viewedEmittedRef.current = data.id;
    viewedAtRef.current = Date.now();
    capture({
      name: "mobile.approval_viewed",
      props: {
        // Stable backend slug, not the user-editable display title.
        service_slug: data.service_slug || "unknown",
        mode: data.approval_mode,
      },
    });
  }, [data]);

  const settingsQuery = useQuery({
    queryKey: ["notifications", "settings"],
    queryFn: mobileApi.getNotificationSettings,
  });

  const decideMutation = useMutation({
    mutationFn: async (decision: "APPROVE" | "DENY") => {
      const durationSec = decision === "APPROVE"
        ? (settingsQuery.data?.grant_expiry_days ?? 30) * 86400
        : undefined;
      const idempotencyKey = createIdempotencyKey("decision", challengeId);
      return mobileApi.submitDecision(challengeId, decision, durationSec);
    },
    onMutate: () => {
      // Snapshot view->tap latency at tap-time (not at server-response
      // time) but DEFER the emit to onSuccess so failures don't
      // overcount decisions.
      const openedAt = viewedAtRef.current;
      const decisionMs = openedAt != null ? Math.max(0, Date.now() - openedAt) : 0;
      return { decisionMs };
    },
    onSuccess: (_, decision, context) => {
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
      void queryClient.invalidateQueries({ queryKey: ["challenge", challengeId] });
      setToast({ message: decision === "APPROVE" ? "Approved" : "Denied", kind: "success" });
      setTimeout(() => {
        navigation.replace("Activity");
      }, 600);
    },
    onError: (err) => {
      setToast({ message: getDecisionErrorMessage(err), kind: "error" });
    },
  });

  if (isLoading) {
    return <FullScreenLoading title="Loading detail..." subtitle="Fetching challenge information" />;
  }

  if (isError || !data) {
    return (
      <ScreenContainer>
        <View style={styles.errorWrap}>
          <Text style={styles.errorTitle}>Challenge Unavailable</Text>
          <Text style={styles.errorMsg}>{getChallengeQueryErrorMessage(error)}</Text>
          <View style={styles.errorActions}>
            <PrimaryButton label="Back" kind="ghost" onPress={() => navigation.goBack()} />
            <PrimaryButton label="Retry" onPress={() => refetch()} />
          </View>
        </View>
      </ScreenContainer>
    );
  }

  const actionState = getChallengeActionState(data);
  const grantDurationLabel = formatGrantDuration(settingsQuery.data?.grant_expiry_days);
  const riskColorMap = { high: colors.riskHigh.text, medium: colors.riskMedium.text, low: colors.riskLow.text };
  const riskColor = riskColorMap[data.risk_level];
  const isGrantMode = data.approval_mode === "grant";

  return (
    <ScreenContainer>
      <ScrollView style={flowStyles.content} contentContainerStyle={[flowStyles.scrollContent, { paddingHorizontal: spacing.xxl }]}>
        <Pressable
          onPress={() => {
            capture({
              name: "ui.mobile_nav_target_opened",
              props: { target: "Activity", source: "back" },
            });
            navigation.goBack();
          }}
          style={styles.backBtn}
        >
          <Text style={styles.backText}>{"\u2190"} Back to Activity</Text>
        </Pressable>

        <Text style={styles.screenTitle}>Approval Detail</Text>
        {isGrantMode && (
          <Text style={styles.screenSub}>
            Review this request before creating reusable access for {grantDurationLabel}.
          </Text>
        )}
        {data.from_org_policy ? (
          <Text style={styles.orgContext}>
            On behalf of {data.org_name ?? "your org"}
          </Text>
        ) : null}

        <View style={styles.detailCard}>
          <Text style={flowStyles.cardTitle}>Request Context</Text>
          <DetailRow label="Action" value={data.action} flowStyles={flowStyles} />
          <DetailRow label="Resource" value={data.resource} flowStyles={flowStyles} />
          <DetailRow label="Client" value={data.request_context.client} flowStyles={flowStyles} />
          <DetailRow label="Risk Level" value={data.risk_level.charAt(0).toUpperCase() + data.risk_level.slice(1)} valueColor={riskColor} flowStyles={flowStyles} />
          <DetailRow label="Status" value={actionState.statusLabel} valueColor={colors.warning} flowStyles={flowStyles} />
          {isGrantMode && <DetailRow label="Grant Duration" value={grantDurationLabel} flowStyles={flowStyles} />}
          {data.from_org_policy ? (
            <DetailRow
              label="Org"
              value={data.org_name ?? "Unnamed org"}
              flowStyles={flowStyles}
            />
          ) : null}
          <DetailRow label="Location" value={data.request_context.location} isLast flowStyles={flowStyles} />
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

const createStyles = (c: ThemeColors) => StyleSheet.create({
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
