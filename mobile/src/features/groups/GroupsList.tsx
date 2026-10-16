import { useEffect } from "react";
import { ActivityIndicator, Pressable, StyleSheet, View } from "react-native";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { ChevronRight, Users } from "lucide-react-native";
import {
  ActionButton,
  Card,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
import { groupsApi } from "../../lib/api/groupsApi";
import { useTheme } from "../../theme/ThemeContext";
import { groupError, groupKeys, useGroupAvailability } from "./useGroupRuntime";
import { livePollInterval } from "../../lib/api/assistantLiveTransport";
import { useGroupsLive } from "./useGroupsLive";

/** Content only: the Agents & Groups hub owns its heading and tabs. */
export function GroupsList({
  onOpenGroup,
  onCreate,
}: {
  onOpenGroup: (id: string) => void;
  onCreate: () => void;
}) {
  const { colors } = useTheme();
  const { epoch, active, online, available } = useGroupAvailability();
  const live = useGroupsLive(epoch, active, available);
  const client = useQueryClient();
  const query = useQuery({
    queryKey: [...groupKeys(epoch), "list"],
    queryFn: ({ signal }) => groupsApi.list(signal),
    enabled: active,
    retry: false,
    refetchInterval: (state) =>
      livePollInterval(
        active &&
          state.state.data?.some((group) => group.working_agent_ids.length)
          ? 3000
          : false,
        live,
      ),
  });
  useEffect(() => {
    const queryKey = [...groupKeys(epoch), "list"];
    if (active) void client.invalidateQueries({ queryKey, exact: true });
    else void client.cancelQueries({ queryKey, exact: true });
  }, [active, epoch, client]);
  return (
    <>
      {!online && (
        <Notice message="You're offline. Reconnect to refresh your groups." />
      )}
      {query.isPending && (
        <ActivityIndicator
          accessibilityLabel="Loading groups"
          color={colors.primary}
        />
      )}
      {query.isError && (
        <>
          <Notice tone="error" message={groupError(query.error)} />
          <ActionButton
            variant="secondary"
            label="Retry groups"
            onPress={() => void query.refetch()}
            disabled={!active}
          />
        </>
      )}
      {query.data?.map((group) => (
        <Pressable
          key={group.id}
          accessibilityRole="button"
          accessibilityLabel={`Open group ${group.name}`}
          onPress={() => onOpenGroup(group.id)}
        >
          <Card>
            <Label
              muted
              style={{
                color: group.working_agent_ids.length
                  ? colors.success
                  : colors.primaryOnTint,
              }}
            >
              {group.working_agent_ids.length
                ? `${group.working_agent_ids.length} working`
                : `${group.members.filter((member) => !member.destroyed).length} agents`}
            </Label>
            <View style={styles.row}>
              <View style={[styles.icon, { backgroundColor: colors.cardSoft }]}>
                <Users size={22} color={colors.primaryOnTint} />
              </View>
              <View style={styles.fill}>
                <Label numberOfLines={1} style={styles.title}>
                  {group.name}
                </Label>
                <Label muted numberOfLines={2}>
                  {group.members.map((member) => member.name).join(" · ")}
                </Label>
                {group.owner?.type === "org" && (
                  <Label muted>{group.owner.name}</Label>
                )}
              </View>
              <ChevronRight size={18} color={colors.textMuted} />
            </View>
          </Card>
        </Pressable>
      ))}
      {!query.isPending && !query.isError && !query.data?.length && (
        <Card>
          <Label>Work together</Label>
          <Label muted>
            Create a group for you and your agents to share one conversation.
          </Label>
        </Card>
      )}
      <ActionButton
        label="Create group"
        onPress={onCreate}
        disabled={!active}
      />
    </>
  );
}
const styles = StyleSheet.create({
  row: { flexDirection: "row", alignItems: "center", gap: 12 },
  fill: { flex: 1, minWidth: 0 },
  icon: {
    width: 40,
    height: 40,
    borderRadius: 12,
    alignItems: "center",
    justifyContent: "center",
  },
  title: { fontSize: 15 },
});
