import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ScrollView, StyleSheet, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { PrimaryButton } from "../../components/PrimaryButton";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { mobileApi } from "../../lib/api/mobileApi";
import { flowStyles } from "../../theme/flowStyles";
import { radius, spacing, typeScale } from "../../theme/designTokens";

type Props = NativeStackScreenProps<RootStackParamList, "RevokeConfirm">;

function formatApprovalRequester(
  requesterType: string,
  requesterLabel: string | null | undefined
): string {
  if (requesterLabel && requesterLabel.trim().length > 0) {
    return `${requesterType} · ${requesterLabel}`;
  }
  return requesterType;
}

export function RevokeConfirmScreen({ navigation, route }: Props) {
  const queryClient = useQueryClient();
  const approvalId = route.params.approvalId;

  const { data } = useQuery({
    queryKey: ["approvals"],
    queryFn: mobileApi.getApprovals,
  });
  const approval = data?.items.find((item) => item.id === approvalId);

  const revokeMutation = useMutation({
    mutationFn: () => mobileApi.revoke(approvalId),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["approvals"] });
      navigation.replace("RevokeSuccess");
    },
  });

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="WARNING" tone="warning" />
        <Text style={flowStyles.title}>Confirm Revoke</Text>
        <Text style={flowStyles.subtitle}>
          This approval will be removed immediately and future requests will require challenge.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Approval</Text>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Service</Text>
            <Text style={flowStyles.rowValue}>{approval?.service_name ?? "--"}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Requester</Text>
            <Text style={flowStyles.rowValue}>
              {approval
                ? formatApprovalRequester(approval.requester_type, approval.requester_label)
                : "--"}
            </Text>
          </View>
          <View style={flowStyles.rowLast}>
            <Text style={flowStyles.rowLabel}>Approval ID</Text>
            <Text style={flowStyles.rowValue}>{approvalId}</Text>
          </View>
        </View>

        <View style={styles.warnCard}>
          <Text style={styles.warnText}>This action takes effect immediately.</Text>
        </View>

        <View style={flowStyles.actionWrap}>
          <PrimaryButton
            label="Confirm Revoke"
            kind="danger"
            disabled={revokeMutation.isPending}
            onPress={() => revokeMutation.mutate()}
          />
          <PrimaryButton
            label="Cancel"
            kind="ghost"
            onPress={() => navigation.goBack()}
          />
        </View>
      </ScrollView>
    </ScreenContainer>
  );
}

const styles = StyleSheet.create({
  warnCard: {
    borderRadius: radius.md,
    borderWidth: 1,
    borderColor: "#7F1D1D",
    backgroundColor: "#2A1217",
    paddingVertical: spacing.lg,
    paddingHorizontal: spacing.xl,
  },
  warnText: {
    color: "#FCA5A5",
    ...typeScale.caption,
    fontSize: 13,
    fontWeight: "600",
  },
});
