import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { useEffect, useState } from "react";
import { Pressable, ScrollView, StyleSheet, Text, View } from "react-native";
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
import {
  getChallengeActionState,
  getChallengeQueryErrorMessage,
  getDecisionErrorMessage,
  getErrorCode,
} from "./challengeUiState";

type Props = NativeStackScreenProps<RootStackParamList, "ChallengeDetail">;

const FIVE_MIN_SEC = 300;
const THIRTY_MIN_SEC = 1800;
const ALWAYS_DURATION_SEC = 315360000;

function formatDuration(sec: number): string {
  if (sec === ALWAYS_DURATION_SEC) return "Always";
  if (sec >= 86400) return `${sec / 86400} day`;
  if (sec >= 3600) return `${sec / 3600} hour`;
  return `${sec / 60} min`;
}

function buildDurationOptions(input: number[]): number[] {
  const filtered = input.filter((item) => item !== FIVE_MIN_SEC && item !== THIRTY_MIN_SEC);
  if (!filtered.includes(ALWAYS_DURATION_SEC)) {
    filtered.push(ALWAYS_DURATION_SEC);
  }
  return filtered;
}

function resolveInitialDuration(allowedDurationsSec: number[], defaultDurationSec: number): number {
  const options = buildDurationOptions(allowedDurationsSec);
  if (options.includes(defaultDurationSec)) {
    return defaultDurationSec;
  }
  return options[0] ?? defaultDurationSec;
}

export function ChallengeDetailScreen({ navigation, route }: Props) {
  const queryClient = useQueryClient();
  const challengeId = route.params.challengeId;
  const [selectedDuration, setSelectedDuration] = useState<number | null>(null);
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

  const decideMutation = useMutation({
    mutationFn: (decision: "APPROVE" | "DENY") =>
      mobileApi.submitDecision(
        challengeId,
        decision,
        selectedDuration ??
          (data
            ? resolveInitialDuration(data.allowed_durations_sec, data.default_duration_sec)
            : undefined)
      ),
    onMutate: () => {
      setToast(null);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["challenges"] });
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
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
    return <FullScreenLoading title="Loading approval details..." subtitle="Syncing context and risk signals" />;
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
          <SectionBadge label="DETAIL" tone="warning" />
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
  const durationOptions = buildDurationOptions(data.allowed_durations_sec);
  const initialDuration = resolveInitialDuration(data.allowed_durations_sec, data.default_duration_sec);
  const effectiveDuration = selectedDuration ?? initialDuration;
  const actionDisabled = decideMutation.isPending || !actionState.canDecide;

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="DETAIL" tone="warning" />
        <Text style={flowStyles.title}>Approval Detail</Text>
        <Text style={flowStyles.subtitle}>
          Review this request with full context before approving.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Request Context</Text>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Action</Text>
            <Text style={flowStyles.rowValue}>{data.action}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Resource</Text>
            <Text style={flowStyles.rowValue}>{data.resource}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>IP</Text>
            <Text style={flowStyles.rowValue}>{data.request_context.ip}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Client</Text>
            <Text style={flowStyles.rowValue}>{data.request_context.client}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Status</Text>
            <Text style={flowStyles.rowValue}>{actionState.statusLabel}</Text>
          </View>
          <View style={flowStyles.rowLast}>
            <Text style={flowStyles.rowLabel}>Location</Text>
            <Text style={flowStyles.rowValue}>{data.request_context.location}</Text>
          </View>
        </View>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Approval Duration</Text>
          <Text style={styles.helper}>Default action is 24-hour approval.</Text>
          <View style={styles.durationWrap}>
            {durationOptions.map((duration) => {
              const active = effectiveDuration === duration;
              return (
                <Pressable
                  key={duration}
                  disabled={actionDisabled}
                  onPress={() => setSelectedDuration(duration)}
                  style={[
                    styles.durationItem,
                    active && styles.durationItemActive,
                    actionDisabled && styles.durationItemDisabled,
                  ]}
                >
                  <Text
                    style={[
                      styles.durationText,
                      active && styles.durationTextActive,
                      actionDisabled && styles.durationTextDisabled,
                    ]}
                  >
                    {formatDuration(duration)}
                  </Text>
                </Pressable>
              );
            })}
          </View>
        </View>
        {actionState.reason ? (
          <View style={styles.stateNotice}>
            <Text style={styles.stateNoticeText}>{actionState.reason}</Text>
          </View>
        ) : null}

        <View style={flowStyles.actionWrap}>
          <PrimaryButton
            label="Approve"
            disabled={actionDisabled}
            onPress={() => decideMutation.mutate("APPROVE")}
          />
          <PrimaryButton
            label="Deny"
            kind="danger"
            disabled={actionDisabled}
            onPress={() => decideMutation.mutate("DENY")}
          />
        </View>
      </ScrollView>
      <ToastOverlay toast={toast} bottom={64} />
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  helper: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    fontSize: 13,
  },
  durationWrap: {
    flexDirection: "row",
    flexWrap: "wrap",
    gap: spacing.sm,
  },
  durationItem: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: mobileTheme.border,
    backgroundColor: mobileTheme.cardSoft,
    paddingVertical: spacing.sm + spacing.xxs,
    paddingHorizontal: spacing.lg,
  },
  durationItemActive: {
    borderColor: "#8B5CF6",
    backgroundColor: "#8B5CF620",
  },
  durationItemDisabled: {
    opacity: 0.6,
  },
  durationText: {
    color: mobileTheme.textSecondary,
    ...typeScale.caption,
    fontWeight: "600",
    fontSize: 13,
  },
  durationTextActive: {
    color: "#D8CCFF",
  },
  durationTextDisabled: {
    color: mobileTheme.textMuted,
  },
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
