import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { RefreshControl, ScrollView, StyleSheet, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { FullScreenLoading } from "../../components/FullScreenLoading";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { ToastKind, ToastOverlay, ToastState } from "../../components/ToastOverlay";
import { mobileApi } from "../../lib/api/mobileApi";
import { mobileTheme } from "../../theme/mobileTheme";
import { flowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "Approvals">;

/** Format backend RFC3339 grant expiry for display (e.g. "Mar 10, 2025" or "in 5 days"). */
function formatGrantExpiry(expiresAt: string): string {
  const date = new Date(expiresAt);
  if (!Number.isFinite(date.getTime())) return expiresAt;
  const now = Date.now();
  const ms = date.getTime() - now;
  const days = Math.round(ms / (24 * 60 * 60 * 1000));
  if (days < 0) return "Expired";
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  if (days < 7) return `In ${days} days`;
  return date.toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });
}

function resolveApprovalsError(error: unknown): string {
  const raw = error instanceof Error ? error.message : "";
  const code = raw.toLowerCase();

  if (
    code.includes("auth_session_missing") ||
    code.includes("unauthorized") ||
    code.includes("invalid_token") ||
    code.includes("request_failed_401")
  ) {
    return "Session expired. Please sign in again.";
  }

  if (code.includes("network request failed") || code.includes("failed to fetch")) {
    return "Unable to reach API server. Pull to refresh.";
  }

  return __DEV__ && raw ? raw : "Failed to load approvals.";
}

export function ApprovalsScreen({ navigation }: Props) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const { data, isLoading, isError, error, isRefetching, refetch } = useQuery({
    queryKey: ["approvals"],
    queryFn: mobileApi.getApprovals,
  });
  const items = Array.isArray(data?.items) ? data.items : [];
  const total = typeof data?.total === "number" ? data.total : items.length;
  const showErrorState = isError && items.length === 0;

  const showToast = (message: string, kind: ToastKind) => {
    setToast({ message, kind });
  };

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  useEffect(() => {
    if (!isError) return;
    showToast(resolveApprovalsError(error), "error");
  }, [isError, error]);

  if (isLoading) {
    return <FullScreenLoading title="Loading approvals..." subtitle="Reading active approved sessions" />;
  }

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
        refreshControl={
          <RefreshControl
            refreshing={isRefetching}
            onRefresh={() => {
              void refetch();
            }}
          />
        }
      >
        <SectionBadge label="ACTIVE APPROVALS" tone="success" />
        <Text style={flowStyles.title}>Approved Sessions</Text>
        <Text style={flowStyles.subtitle}>
          Revoke completed approvals to force future requests back to challenge.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Current Status</Text>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Active Approvals</Text>
            <Text style={flowStyles.rowValue}>{total}</Text>
          </View>
          <View style={flowStyles.rowLast}>
            <Text style={flowStyles.rowLabel}>Last Sync</Text>
            <Text style={flowStyles.rowValue}>Just now</Text>
          </View>
        </View>

        <View style={flowStyles.card}>
          {showErrorState ? (
            <View style={styles.errorBox}>
              <Text style={styles.errorTitle}>Approvals unavailable</Text>
              <Text style={styles.errorSub}>Pull down or tap retry after fixing connection.</Text>
              <PrimaryButton
                label="Retry"
                kind="ghost"
                onPress={() => {
                  void refetch();
                }}
              />
            </View>
          ) : (
            <>
              {items.map((item) => (
                <View key={item.id} style={styles.itemCard}>
                  <Text style={styles.itemTitle}>{item.action}</Text>
                  <Text style={styles.itemSub}>{item.resource}</Text>
                  <Text style={styles.itemTime}>
                    Grant expires: {formatGrantExpiry(item.expires_at)}
                  </Text>
                  <PrimaryButton
                    label="Revoke"
                    kind="danger"
                    onPress={() => navigation.navigate("RevokeConfirm", { approvalId: item.id })}
                  />
                </View>
              ))}
              {items.length === 0 ? (
                <View style={styles.emptyBox}>
                  <View style={styles.emptyIconWrap}>
                    <Text style={styles.emptyIcon}>✓</Text>
                  </View>
                  <Text style={styles.emptyTitle}>No active approvals</Text>
                  <Text style={styles.emptySub}>
                    New approvals will appear here after successful challenge decisions.
                  </Text>
                </View>
              ) : null}
            </>
          )}
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  itemCard: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: mobileTheme.border,
    backgroundColor: mobileTheme.cardSoft,
    padding: spacing.lg,
    gap: spacing.sm,
  },
  itemTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
  },
  itemSub: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    fontSize: 13,
  },
  itemTime: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
  },
  errorBox: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: "#F8717140",
    backgroundColor: "#7F1D1D22",
    padding: spacing.xxl,
    gap: spacing.sm,
    alignItems: "center",
  },
  errorTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
    textAlign: "center",
  },
  errorSub: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
    textAlign: "center",
    lineHeight: 18,
  },
  emptyBox: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: mobileTheme.border,
    backgroundColor: mobileTheme.cardSoft,
    padding: spacing.xxl,
    gap: spacing.sm,
    alignItems: "center",
  },
  emptyIconWrap: {
    width: 34,
    height: 34,
    borderRadius: 17,
    borderWidth: 1,
    borderColor: "#34D39970",
    backgroundColor: "#064E3B55",
    alignItems: "center",
    justifyContent: "center",
  },
  emptyIcon: {
    color: "#34D399",
    ...typeScale.bodyStrong,
    lineHeight: 18,
  },
  emptyTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
    textAlign: "center",
  },
  emptySub: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
    textAlign: "center",
    lineHeight: 18,
  },
});
