import { useState } from "react";
import { View } from "react-native";
import { useQuery } from "@tanstack/react-query";
import type { NativeStackScreenProps } from "@react-navigation/native-stack";
import type { WorkspaceContentParamList } from "../../app/navigationTypes";
import {
  ActionButton,
  Card,
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { WorkspaceSelectionField } from "../../components/workspace/WorkspaceSelectionField";
import { StatusBadge } from "../../components/StatusBadge";
import { machinesApi } from "../../lib/api/machinesApi";
import { machineError, machineKeys, useMachineRuntime } from "./MachineUI";
import { machineStyles } from "./MachinePresentation";

export function MachineActivityScreen({
  route,
}: NativeStackScreenProps<WorkspaceContentParamList, "MachineActivity">) {
  const runtime = useMachineRuntime();
  const [filter, setFilter] = useState("");
  const [before, setBefore] = useState<string>();
  const query = useQuery({
    queryKey: [
      ...machineKeys(runtime.epoch),
      "activity",
      route.params.machineId,
      filter,
      before,
    ],
    queryFn: ({ signal }) =>
      machinesApi.activity(
        route.params.machineId,
        route.params.ownerId,
        filter,
        before,
        signal,
      ),
    enabled: runtime.active,
    retry: false,
  });
  return (
    <WorkspacePage
      title="Machine activity"
      showHeading={false}
      refreshing={query.isRefetching}
      onRefresh={() => {
        if (runtime.active) void query.refetch();
      }}
    >
      <Label muted style={machineStyles.metadata}>
        Metadata only; command contents, credentials and desktop frames are not
        included.
      </Label>
      <WorkspaceSelectionField
        label="Agent filter"
        value={[filter]}
        allowEmpty={false}
        options={[
          { value: "", label: "All agents" },
          { value: "unattributed", label: "Unattributed" },
          ...(query.data?.agents ?? []).map((agent) => ({
            value: agent.id,
            label: agent.display_name ?? agent.name,
          })),
        ]}
        onChange={(values) => {
          setFilter(values[0] ?? "");
          setBefore(undefined);
        }}
      />
      {query.isPending && <Label muted>Loading machine activity…</Label>}
      {query.error && (
        <Notice tone="error" message={machineError(query.error)} />
      )}
      {!query.error &&
        query.data?.entries.map((entry) => (
          <Card key={entry.id}>
            <View style={machineStyles.row}>
              <Label numberOfLines={1} style={machineStyles.name}>
                {entry.action}
              </Label>
              <StatusBadge
                label={entry.outcome}
                variant={
                  entry.outcome === "completed"
                    ? "decisionApproved"
                    : entry.outcome === "failed" || entry.outcome === "denied"
                      ? "decisionDenied"
                      : "decisionExpired"
                }
              />
            </View>
            <Label muted style={machineStyles.metadata}>
              {query.data!.agents.find((agent) => agent.id === entry.agent_id)
                ?.display_name ??
                query.data!.agents.find((agent) => agent.id === entry.agent_id)
                  ?.name ??
                "Unattributed"}
              {" · "}
              {entry.created_at}
            </Label>
            <View style={machineStyles.badges}>
              {entry.duration_ms != null && (
                <StatusBadge
                  label={`${entry.duration_ms} ms`}
                  variant="orgChip"
                />
              )}
              {entry.exit_code != null && (
                <StatusBadge
                  label={`Exit code: ${entry.exit_code}`}
                  variant="orgChip"
                />
              )}
            </View>
          </Card>
        ))}
      {query.data && !query.error && !query.data.entries.length && (
        <Label muted>No activity matches this filter.</Label>
      )}
      {query.data?.next_cursor && !query.error && (
        <ActionButton
          compact
          label="Older activity"
          variant="secondary"
          disabled={!runtime.active || query.isFetching}
          onPress={() => setBefore(query.data!.next_cursor!)}
        />
      )}
      {before && (
        <ActionButton
          compact
          label="Latest activity"
          variant="secondary"
          onPress={() => setBefore(undefined)}
        />
      )}
    </WorkspacePage>
  );
}
