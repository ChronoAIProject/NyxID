import { useEffect, useMemo, useState } from "react";
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
    onSuccess: (_, decision) => {
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
        <Pressable onPress={() => navigation.goBack()} style={styles.backBtn}>
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
          <DetailRow label="Risk Level" value={data.risk_level.toUpperCase()} valueColor={riskColor} flowStyles={flowStyles} />
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
    fontSize: 12,
    color: c.textMuted,
  },
  screenTitle: {
    fontSize: 22,
    fontWeight: "700",
    color: c.textPrimary,
    fontFamily: "SpaceGrotesk_700Bold",
  },
  screenSub: {
    fontSize: 13,
    color: c.textSecondary,
    lineHeight: 20,
  },
  orgContext: {
    fontSize: 12,
    fontWeight: "600",
    color: c.textSecondary,
    letterSpacing: 0.3,
  },
  stateNotice: {
    borderRadius: radius.sm,
    backgroundColor: "rgba(245,158,11,0.1)",
    borderWidth: 1,
    borderColor: "rgba(245,158,11,0.25)",
    padding: spacing.lg,
  },
  stateNoticeText: {
    fontSize: 13,
    color: c.warning,
    lineHeight: 20,
  },
  detailCard: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: c.border,
    backgroundColor: c.cardSoft,
    padding: spacing.lg,
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
    fontSize: 18,
    fontWeight: "700",
    color: c.textPrimary,
  },
  errorMsg: {
    fontSize: 14,
    color: c.textSecondary,
    textAlign: "center",
  },
  errorActions: {
    flexDirection: "row",
    gap: spacing.md,
  },
});
