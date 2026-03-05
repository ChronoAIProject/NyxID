import { useEffect, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { Pressable, RefreshControl, ScrollView, StyleSheet, Text, View } from "react-native";
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
import { formatGrantExpiryFromCreatedAt } from "./challengeUiState";

type Props = NativeStackScreenProps<RootStackParamList, "Inbox">;

function resolveChallengesError(error: unknown): string {
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

  return __DEV__ && raw ? raw : "Failed to load challenges.";
}

export function ChallengesInboxScreen({ navigation }: Props) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const { data, isLoading, isError, error, isRefetching, refetch } = useQuery({
    queryKey: ["challenges", "pending"],
    queryFn: mobileApi.getChallenges,
  });
  const { data: notificationSettings } = useQuery({
    queryKey: ["notifications", "settings"],
    queryFn: mobileApi.getNotificationSettings,
  });
  const items = Array.isArray(data?.items) ? data.items : [];
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
    showToast(resolveChallengesError(error), "error");
  }, [isError, error]);

  if (isLoading) {
    return <FullScreenLoading title="Loading pending challenges..." subtitle="Fetching the latest approval requests" />;
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
        <SectionBadge label="PENDING" tone="warning" />
        <Text style={flowStyles.title}>Pending Challenges</Text>
        <Text style={flowStyles.subtitle}>
          Review and decide high-risk actions waiting for approval.
        </Text>

        <View style={flowStyles.card}>
          {showErrorState ? (
            <View style={styles.errorBox}>
              <Text style={styles.errorTitle}>Challenges unavailable</Text>
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
                <Pressable
                  key={item.id}
                  style={styles.challengeCard}
                  onPress={() => navigation.navigate("ChallengeDetail", { challengeId: item.id })}
                >
                  <View style={styles.challengeHeader}>
                    <Text style={styles.challengeTitle}>{item.action}</Text>
                    <View
                      style={[
                        styles.riskBadge,
                        item.risk_level === "high" ? styles.riskHigh : styles.riskMedium,
                      ]}
                    >
                      <Text style={styles.riskText}>{item.risk_level.toUpperCase()}</Text>
                    </View>
                  </View>
                  <Text style={styles.challengeResource}>{item.resource}</Text>
                  <Text style={styles.challengeExpire}>
                    Grant expires:{" "}
                    {formatGrantExpiryFromCreatedAt(
                      item.created_at,
                      notificationSettings?.grant_expiry_days
                    )}
                  </Text>
                </Pressable>
              ))}
              {items.length === 0 ? (
                <View style={styles.emptyBox}>
                  <View style={styles.emptyIconWrap}>
                    <Text style={styles.emptyIcon}>!</Text>
                  </View>
                  <Text style={styles.emptyTitle}>No pending challenges</Text>
                  <Text style={styles.emptySub}>
                    New high-risk requests will appear here for your review.
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
  challengeCard: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: mobileTheme.border,
    backgroundColor: mobileTheme.cardSoft,
    padding: spacing.lg,
    gap: spacing.xs + spacing.xxs,
  },
  challengeHeader: {
    flexDirection: "row",
    justifyContent: "space-between",
    alignItems: "center",
    gap: spacing.sm,
  },
  challengeTitle: {
    color: mobileTheme.textPrimary,
    ...typeScale.bodyStrong,
    flex: 1,
  },
  challengeResource: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    fontSize: 13,
  },
  challengeExpire: {
    color: mobileTheme.textMuted,
    ...typeScale.caption,
  },
  riskBadge: {
    borderRadius: radius.sm,
    borderWidth: 1,
    paddingHorizontal: spacing.sm,
    paddingVertical: spacing.xs - spacing.xxs,
  },
  riskHigh: {
    borderColor: "#F8717140",
    backgroundColor: "#7F1D1D30",
  },
  riskMedium: {
    borderColor: "#F59E0B40",
    backgroundColor: "#78350F30",
  },
  riskText: {
    color: "#FCA5A5",
    ...typeScale.overline,
    fontWeight: "700",
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
    borderColor: "#F59E0B70",
    backgroundColor: "#78350F30",
    alignItems: "center",
    justifyContent: "center",
  },
  emptyIcon: {
    color: "#F59E0B",
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
