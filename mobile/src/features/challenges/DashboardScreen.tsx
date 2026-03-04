import { useQuery } from "@tanstack/react-query";
import { NativeStackScreenProps } from "@react-navigation/native-stack";
import { ScrollView, Text, View } from "react-native";
import { RootStackParamList } from "../../app/AppNavigator";
import { MobileStatusBar } from "../../components/MobileStatusBar";
import { ScreenContainer } from "../../components/ScreenContainer";
import { SectionBadge } from "../../components/SectionBadge";
import { mobileApi } from "../../lib/api/mobileApi";
import { flowStyles } from "../../theme/flowStyles";

type Props = NativeStackScreenProps<RootStackParamList, "Dashboard">;

export function DashboardScreen({ navigation: _navigation }: Props) {
  const { data: challengeData } = useQuery({
    queryKey: ["challenges", "pending"],
    queryFn: mobileApi.getChallenges,
  });
  const { data: approvalData } = useQuery({
    queryKey: ["approvals"],
    queryFn: mobileApi.getApprovals,
  });

  return (
    <ScreenContainer>
      <MobileStatusBar />
      <ScrollView
        style={flowStyles.content}
        contentContainerStyle={flowStyles.scrollContent}
        showsVerticalScrollIndicator={false}
      >
        <SectionBadge label="SECURE" tone="success" />
        <Text style={flowStyles.title}>Dashboard</Text>
        <Text style={flowStyles.subtitle}>
          Monitor approval health and act on risky requests.
        </Text>

        <View style={flowStyles.card}>
          <Text style={flowStyles.cardTitle}>Security Status</Text>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Pending Challenges</Text>
            <Text style={flowStyles.rowValue}>{challengeData?.total ?? 0}</Text>
          </View>
          <View style={flowStyles.row}>
            <Text style={flowStyles.rowLabel}>Active Approvals</Text>
            <Text style={flowStyles.rowValue}>{approvalData?.total ?? 0}</Text>
          </View>
          <View style={flowStyles.rowLast}>
            <Text style={flowStyles.rowLabel}>Last Refresh</Text>
            <Text style={flowStyles.rowValue}>Just now</Text>
          </View>
        </View>

      </ScrollView>
    </ScreenContainer>
  );
}
