import { Pressable, StyleSheet, View } from "react-native";
import { Plus } from "lucide-react-native";
import { SegmentControl } from "../../components/SegmentControl";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { AgentsScreen } from "./AgentsScreen";
import { GroupsList } from "../groups/GroupsList";

type AgentsProps = NonNullable<Parameters<typeof AgentsScreen>[0]>;

export function AgentsGroupsScreen({
  tab,
  onTabChange,
  onOpenGroup,
  onCreateGroup,
  onCreateAgent,
  ...agentsProps
}: AgentsProps & {
  tab: "agents" | "groups";
  onTabChange: (tab: "agents" | "groups") => void;
  onOpenGroup: (id: string) => void;
  onCreateGroup: () => void;
  onCreateAgent: () => void;
}) {
  const { colors } = useTheme();
  const { isConnected } = useNetworkStatus();
  const tabs = (
    <View style={styles.tabRow}>
      <View style={styles.fill}>
        <SegmentControl
          segments={[{ label: "Agents" }, { label: "Crews" }]}
          activeIndex={tab === "agents" ? 0 : 1}
          onPress={(index) => onTabChange(index === 0 ? "agents" : "groups")}
          style={styles.tabs}
        />
      </View>
      <Pressable
        accessibilityRole="button"
        accessibilityLabel={tab === "agents" ? "New agent" : "New crew"}
        accessibilityState={{ disabled: !isConnected }}
        disabled={!isConnected}
        onPress={tab === "agents" ? onCreateAgent : onCreateGroup}
        style={[
          styles.createButton,
          {
            backgroundColor: colors.primaryTone.bg,
            opacity: isConnected ? 1 : 0.4,
          },
        ]}
      >
        <Plus size={21} color={colors.primaryOnTint} />
      </Pressable>
    </View>
  );
  if (tab === "groups") {
    return (
      <WorkspacePage
        title="Crews"
        subtitle="Bring your agents together in one conversation"
      >
        {tabs}
        <GroupsList onOpenGroup={onOpenGroup} onCreate={onCreateGroup} />
      </WorkspacePage>
    );
  }
  return (
    <AgentsScreen {...agentsProps} onCreate={onCreateAgent} listHeader={tabs} />
  );
}

const styles = StyleSheet.create({
  tabs: { marginBottom: 0 },
  fill: { flex: 1 },
  tabRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  createButton: {
    width: 48,
    height: 48,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
});
