import { Pressable, StyleSheet, View } from "react-native";
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
  ...agentsProps
}: AgentsProps & {
  tab: "agents" | "groups";
  onTabChange: (tab: "agents" | "groups") => void;
  onOpenGroup: (id: string) => void;
  onCreateGroup: () => void;
}) {
  const tabs = <AgentsGroupsTabs value={tab} onChange={onTabChange} />;
  if (tab === "groups") {
    return (
      <WorkspacePage
        title="Groups"
        subtitle="Bring your agents together in one conversation"
      >
        {tabs}
        <GroupsList onOpenGroup={onOpenGroup} onCreate={onCreateGroup} />
      </WorkspacePage>
    );
  }
  return <AgentsScreen {...agentsProps} listHeader={tabs} />;
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
          accessibilityLabel={tab === "agents" ? "Agents" : "Groups"}
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
            {tab === "agents" ? "Agents" : "Groups"}
          </Label>
        </Pressable>
      ))}
    </View>
  );
}

const styles = StyleSheet.create({
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
