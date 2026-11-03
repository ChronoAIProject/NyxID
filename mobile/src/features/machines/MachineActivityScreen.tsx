import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import type { ContentScreenProps } from "../../app/navigationTypes";
import {
  SettingsCheckRow,
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import {
  Label,
  Notice,
  WorkspacePage,
} from "../../components/workspace/WorkspaceUI";
import { machinesApi } from "../../lib/api/machinesApi";
import { machineError, machineKeys, useMachineRuntime } from "./MachineUI";
import { formatMachineDate, machinePageContent } from "./MachinePresentation";

const ALL_AGENTS = "";
const UNATTRIBUTED = "unattributed";

export function activityOutcomeTone(outcome: string) {
  return outcome === "failed" || outcome === "denied" ? "danger" : "default";
}

export function activityDetailLine(
  agent: string,
  createdAt: string,
  durationMs?: number | null,
  exitCode?: number | null,
) {
  return [
    agent,
    formatMachineDate(createdAt),
    durationMs != null ? `${durationMs} ms` : undefined,
    exitCode != null ? `Exit code: ${exitCode}` : undefined,
  ]
    .filter(Boolean)
    .join(" · ");
}

export function MachineActivityScreen({
  route,
  navigation,
}: ContentScreenProps<"MachineActivity">) {
  const runtime = useMachineRuntime();
  const filter = route.params.agent ?? ALL_AGENTS;
  // A page cursor belongs to the filter it was read with.
  const [page, setPage] = useState<{ filter: string; before: string }>();
  const before = page?.filter === filter ? page.before : undefined;
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
  const agents = (query.data?.agents ?? []).map((agent) => ({
    id: agent.id,
    label: agent.display_name ?? agent.name,
  }));
  const agentLabel = (id?: string | null) =>
    agents.find((agent) => agent.id === id)?.label ?? "Unattributed";
  const filterLabel =
    filter === ALL_AGENTS
      ? "All agents"
      : filter === UNATTRIBUTED
        ? "Unattributed"
        : (agents.find((agent) => agent.id === filter)?.label ??
          "Selected agent");
  return (
    <WorkspacePage
      title="Machine activity"
      showHeading={false}
      refreshing={query.isRefetching}
      onRefresh={() => {
        if (runtime.active) void query.refetch();
      }}
      contentStyle={machinePageContent}
    >
      <SettingsGroup
        inset={false}
        footer="Metadata only; command contents, credentials and desktop frames are not included."
      >
        <SettingsRow
          label="Agent"
          value={filterLabel}
          accessibilityLabel={`Agent filter, ${filterLabel}`}
          onPress={() =>
            navigation.push("MachineActivityFilter", {
              machineId: route.params.machineId,
              ownerId: route.params.ownerId,
              agent: filter,
              agents,
            })
          }
        />
      </SettingsGroup>
      {query.isPending && <Label muted>Loading machine activity…</Label>}
      {query.error && (
        <Notice tone="error" message={machineError(query.error)} />
      )}
      {query.data && !query.error && (
        <SettingsGroup
          title={before ? "Older activity" : "Recent activity"}
          inset={false}
        >
          {query.data.entries.map((entry) => {
            const agent = agentLabel(entry.agent_id);
            return (
              <SettingsRow
                key={entry.id}
                label={entry.action}
                detail={activityDetailLine(
                  agent,
                  entry.created_at,
                  entry.duration_ms,
                  entry.exit_code,
                )}
                value={entry.outcome}
                valueTone={activityOutcomeTone(entry.outcome)}
                onPress={() =>
                  navigation.push("MachineActivityDetail", {
                    id: entry.id,
                    action: entry.action,
                    outcome: entry.outcome,
                    agent,
                    createdAt: entry.created_at,
                    durationMs: entry.duration_ms ?? undefined,
                    exitCode: entry.exit_code ?? undefined,
                  })
                }
              />
            );
          })}
          {!query.data.entries.length && (
            <SettingsRow label="No activity matches this filter." />
          )}
        </SettingsGroup>
      )}
      {!query.error && (query.data?.next_cursor || before) && (
        <SettingsGroup inset={false}>
          {query.data?.next_cursor && (
            <SettingsRow
              tone="action"
              label="Older Activity"
              disabled={!runtime.active || query.isFetching}
              onPress={() =>
                setPage({ filter, before: query.data!.next_cursor! })
              }
            />
          )}
          {before && (
            <SettingsRow
              tone="action"
              label="Latest Activity"
              onPress={() => setPage(undefined)}
            />
          )}
        </SettingsGroup>
      )}
    </WorkspacePage>
  );
}

/** Choosing the agent filter: a checkmark list, since agents are open-ended. */
export function MachineActivityFilterScreen({
  route,
  navigation,
}: ContentScreenProps<"MachineActivityFilter">) {
  const { machineId, ownerId, agent, agents } = route.params;
  const choose = (value: string) =>
    navigation.popTo(
      "MachineActivity",
      { machineId, ownerId, agent: value },
      { merge: true },
    );
  return (
    <WorkspacePage
      title="Agent"
      showHeading={false}
      contentStyle={machinePageContent}
    >
      <SettingsGroup inset={false}>
        <SettingsCheckRow
          label="All agents"
          selected={agent === ALL_AGENTS}
          onPress={() => choose(ALL_AGENTS)}
        />
        <SettingsCheckRow
          label="Unattributed"
          selected={agent === UNATTRIBUTED}
          onPress={() => choose(UNATTRIBUTED)}
        />
      </SettingsGroup>
      {agents.length ? (
        <SettingsGroup title="Agents" inset={false}>
          {agents.map((row) => (
            <SettingsCheckRow
              key={row.id}
              label={row.label}
              selected={agent === row.id}
              onPress={() => choose(row.id)}
            />
          ))}
        </SettingsGroup>
      ) : null}
    </WorkspacePage>
  );
}

/** One metadata-only activity entry. */
export function MachineActivityDetailScreen({
  route,
}: ContentScreenProps<"MachineActivityDetail">) {
  const entry = route.params;
  return (
    <WorkspacePage
      title={entry.action}
      showHeading={false}
      contentStyle={machinePageContent}
    >
      <SettingsGroup
        inset={false}
        footer="Metadata only; command contents, credentials and desktop frames are not included."
      >
        <SettingsRow label="Action" value={entry.action} />
        <SettingsRow
          label="Outcome"
          value={entry.outcome}
          valueTone={activityOutcomeTone(entry.outcome)}
        />
        <SettingsRow label="Agent" value={entry.agent} />
        <SettingsRow label="Time" value={formatMachineDate(entry.createdAt)} />
        {entry.durationMs != null ? (
          <SettingsRow label="Duration" value={`${entry.durationMs} ms`} />
        ) : null}
        {entry.exitCode != null ? (
          <SettingsRow label="Exit code" value={String(entry.exitCode)} />
        ) : null}
      </SettingsGroup>
      <SettingsGroup title="Entry ID" inset={false}>
        <SettingsRow label={entry.id} mono selectable />
      </SettingsGroup>
    </WorkspacePage>
  );
}
