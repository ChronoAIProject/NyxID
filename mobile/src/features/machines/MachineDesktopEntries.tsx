import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { Notice } from "../../components/workspace/WorkspaceUI";
import { requestJson } from "../../lib/api/http";
import { desktopEntryParams, desktopMetadataSchema } from "./desktopEntry";
import type { MachineDesktopParams } from "./desktop/desktopRoutes";
import { machineError, machineKeys, useMachineRuntime } from "./MachineUI";

const contextSchema = z.object({
  context_id: z.string(),
  agent_name: z.string(),
  actor_label: z.string(),
  group_id: z.string().nullable(),
});

export function MachineDesktopEntries({
  nodeId,
  conversationId,
  onOpen,
}: {
  nodeId: string;
  conversationId?: string;
  onOpen: (params: MachineDesktopParams) => void;
}) {
  const runtime = useMachineRuntime();
  const query = useQuery({
    queryKey: [
      ...machineKeys(runtime.epoch),
      "desktop-entries",
      nodeId,
      conversationId,
    ],
    queryFn: async ({ signal }) => {
      const params = new URLSearchParams();
      if (conversationId) params.set("conversation_id", conversationId);
      const rows = desktopMetadataSchema
        .array()
        .parse(
          await requestJson(
            `/assistant/nyxagent/machines${params.size ? `?${params}` : ""}`,
            { signal },
          ),
        );
      const contexts = conversationId
        ? []
        : contextSchema
            .array()
            .parse(
              await requestJson(
                `/assistant/nyxagent/machines/${encodeURIComponent(nodeId)}/contexts`,
                { signal },
              ),
            );
      return {
        rows: rows.filter(
          (row) =>
            row.node_id === nodeId &&
            (!conversationId || row.conversation_id === conversationId),
        ),
        contexts,
      };
    },
    enabled: runtime.active,
    retry: false,
    refetchInterval: runtime.active ? 10_000 : false,
    refetchIntervalInBackground: false,
  });
  const displayName = (display: "secure" | "dev") =>
    display === "dev" ? "Developer" : "Secure";
  if (query.error)
    return <Notice tone="error" message={machineError(query.error)} />;
  return (
    <SettingsGroup
      title="Desktop"
      inset={false}
      footer="Viewing leaves the task running. Desktop availability is checked when opened."
    >
      {query.isPending && <SettingsRow label="Checking desktop sessions…" />}
      {query.data?.rows.map((row) => (
        <SettingsRow
          key={row.session_id}
          label={`${displayName(row.display)} desktop`}
          value={row.status}
          accessibilityLabel={`View desktop · ${displayName(row.display)} · ${row.status}`}
          accessibilityHint="Opens the live desktop viewer"
          disabled={!runtime.active}
          onPress={() => onOpen(desktopEntryParams(row, conversationId))}
        />
      ))}
      {!conversationId &&
        query.data?.contexts.flatMap((context) =>
          (["secure", "dev"] as const).map((display) => {
            if (
              query.data.rows.some(
                (row) =>
                  row.context_id === context.context_id &&
                  row.display === display,
              )
            )
              return null;
            return (
              <SettingsRow
                key={`${context.context_id}:${display}`}
                label={context.agent_name}
                detail={`${context.actor_label} · ${displayName(display)} desktop`}
                accessibilityLabel={`View desktop · ${context.agent_name} · ${context.actor_label} · ${displayName(display)}`}
                accessibilityHint="Opens the live desktop viewer"
                disabled={!runtime.active}
                onPress={() =>
                  onOpen({ nodeId, contextId: context.context_id, display })
                }
              />
            );
          }),
        )}
      {query.data &&
        !query.data.rows.length &&
        !query.data.contexts.length &&
        (conversationId ? (
          <SettingsRow label="No desktop is linked to this conversation." />
        ) : (
          <SettingsRow
            label="View Desktop"
            accessibilityHint="Opens the live desktop viewer"
            disabled={!runtime.active}
            onPress={() => onOpen({ nodeId })}
          />
        ))}
    </SettingsGroup>
  );
}
