import { Pressable, StyleSheet, View } from "react-native";
import { Plus } from "lucide-react-native";
import { useNetworkStatus } from "../../hooks/useNetworkStatus";
import { Label, WorkspacePage } from "../../components/workspace/WorkspaceUI";
import { useTheme } from "../../theme/ThemeContext";
import { fonts } from "../../theme/designTokens";
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
        <AgentsGroupsTabs value={tab} onChange={onTabChange} />
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

function AgentsGroupsTabs({
  value,
  onChange,
}: {
  value: "agents" | "groups";
  onChange: (tab: "agents" | "groups") => void;
}) {
  const { colors } = useTheme();
  return (
    <View
      accessibilityRole="tablist"
      style={[
        styles.tabs,
        { backgroundColor: colors.cardSoft, borderColor: colors.border },
      ]}
    >
      {(["agents", "groups"] as const).map((tab) => (
        <Pressable
          key={tab}
          accessibilityRole="tab"
          accessibilityLabel={tab === "agents" ? "Agents" : "Crews"}
          accessibilityState={{ selected: value === tab }}
          onPress={() => onChange(tab)}
          style={[
            styles.tab,
            {
              backgroundColor: value === tab ? colors.navActive : "transparent",
            },
          ]}
        >
          <Label
            style={{
              fontFamily: fonts.bodySemi,
              color:
                value === tab ? colors.primaryOnTint : colors.textSecondary,
            }}
          >
            {tab === "agents" ? "Agents" : "Crews"}
          </Label>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  tabRow: { flexDirection: "row", alignItems: "center", gap: 12 },
  createButton: {
    width: 48,
    height: 48,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  tabs: {
    flexDirection: "row",
    padding: 4,
    gap: 4,
    borderRadius: 12,
    borderWidth: 1,
  },
  tab: {
    flex: 1,
    minHeight: 44,
    alignItems: "center",
    justifyContent: "center",
    borderRadius: 8,
  },
});
