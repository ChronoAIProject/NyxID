import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useEffect, useState } from "react";
import { ScrollView, StyleSheet, Text, View } from "react-native";
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
import { typeScale } from "../../theme/designTokens";
import {
  getChallengeActionState,
  getChallengeQueryErrorMessage,
  getDecisionErrorMessage,
  getErrorCode,
} from "./challengeUiState";

type Props = NativeStackScreenProps<RootStackParamList, "ChallengeMinimal">;

export function ChallengeMinimalScreen({ navigation, route }: Props) {
  const queryClient = useQueryClient();
  const challengeId = route.params.challengeId;
  const [toast, setToast] = useState<ToastState | null>(null);

  const showToast = (message: string, kind: ToastKind) => {
    setToast({ message, kind });
  };

  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(null), 2400);
    return () => clearTimeout(timer);
  }, [toast]);

  const { data, isLoading, isError, error, refetch } = useQuery({
    queryKey: ["challenge", challengeId],
    queryFn: () => mobileApi.getChallengeById(challengeId),
  });

  const approveMutation = useMutation({
    mutationFn: () =>
      mobileApi.submitDecision(challengeId, "APPROVE", data?.default_duration_sec ?? 86400),
    onMutate: () => {
      setToast(null);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["challenges"] });
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
      navigation.replace("Approvals");
    },
    onError: (mutationError) => {
      showToast(getDecisionErrorMessage(mutationError), "error");
      const code = getErrorCode(mutationError);
      if (code === "already_decided" || code === "challenge_not_found") {
        void queryClient.invalidateQueries({ queryKey: ["challenge", challengeId] });
      }
    },
  });

  const denyMutation = useMutation({
    mutationFn: () => mobileApi.submitDecision(challengeId, "DENY"),
    onMutate: () => {
      setToast(null);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["challenges"] });
      navigation.replace("Dashboard");
    },
    onError: (mutationError) => {
      showToast(getDecisionErrorMessage(mutationError), "error");
      const code = getErrorCode(mutationError);
      if (code === "already_decided" || code === "challenge_not_found") {
        void queryClient.invalidateQueries({ queryKey: ["challenge", challengeId] });
      }
    },
  });

  if (isLoading) {
    return <FullScreenLoading title="Loading approval content..." subtitle="Fetching challenge details" />;
  }

  if (isError || !data) {
    return (
      <ScreenContainer>
        <MobileStatusBar />
        <ScrollView
          style={flowStyles.content}
          contentContainerStyle={flowStyles.scrollContent}
          showsVerticalScrollIndicator={false}
        >
          <SectionBadge label="CHALLENGE" tone="warning" />
          <Text style={flowStyles.title}>Challenge Unavailable</Text>
          <Text style={flowStyles.subtitle}>{getChallengeQueryErrorMessage(error)}</Text>
          <View style={flowStyles.actionWrap}>
            <PrimaryButton label="Back to Inbox" onPress={() => navigation.replace("Inbox")} />
            <PrimaryButton
              label="Retry"
              kind="ghost"
              onPress={() => {
                void refetch();
              }}
            />
          </View>
        </ScrollView>
      </ScreenContainer>
    );
  }

  const actionState = getChallengeActionState(data);
  const actionsDisabled =
    approveMutation.isPending || denyMutation.isPending || !actionState.canDecide;

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="CHALLENGE" tone="warning" />
        <Text style={flowStyles.title}>Approve This Request?</Text>
        <Text style={flowStyles.subtitle}>
          Default action is 24-hour approval for this same request pattern.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Action Summary</Text>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Action</Text>
            <Text style={flowStyles.rowValue}>{data.action}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Resource</Text>
            <Text style={flowStyles.rowValue}>{data.resource}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Status</Text>
            <Text style={flowStyles.rowValue}>{actionState.statusLabel}</Text>
          </View>
          <View style={flowStyles.rowLast}>
            <Text style={flowStyles.rowLabel}>Expires</Text>
            <Text style={flowStyles.rowValue}>{data.expires_at}</Text>
          </View>
        </View>
        {actionState.reason ? (
          <View style={styles.stateNotice}>
            <Text style={styles.stateNoticeText}>{actionState.reason}</Text>
          </View>
        ) : null}

        <View style={flowStyles.actionWrap}>
          <PrimaryButton
            label="Approve 24h"
            disabled={actionsDisabled}
            onPress={() => approveMutation.mutate()}
          />
          <PrimaryButton
            label="More Options"
            kind="ghost"
            disabled={actionsDisabled}
            onPress={() => navigation.navigate("ChallengeOptions", { challengeId })}
          />
          <PrimaryButton
            label="Deny"
            kind="danger"
            disabled={actionsDisabled}
            onPress={() => denyMutation.mutate()}
          />
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  stateNotice: {
    borderWidth: 1,
    borderColor: mobileTheme.borderSoft,
    backgroundColor: mobileTheme.cardSoft,
    borderRadius: 14,
    paddingHorizontal: 14,
    paddingVertical: 12,
    marginBottom: 12,
  },
  stateNoticeText: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
  },
});
