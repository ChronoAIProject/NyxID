import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  ActionButton,
  Label,
  Notice,
} from "../../components/workspace/WorkspaceUI";
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
  return (
    <>
      {query.isPending && <Label muted>Checking desktop sessions…</Label>}
      {query.error && (
        <Notice tone="error" message={machineError(query.error)} />
      )}
      {!query.error &&
        query.data?.rows.map((row) => (
          <ActionButton
            compact
            key={row.session_id}
            label={`View desktop · ${row.display === "dev" ? "Developer" : "Secure"} · ${row.status}`}
            variant="secondary"
            disabled={!runtime.active}
            onPress={() => onOpen(desktopEntryParams(row, conversationId))}
          />
        ))}
      {!query.error &&
        !conversationId &&
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
              <ActionButton
                compact
                key={`${context.context_id}:${display}`}
                label={`View desktop · ${context.agent_name} · ${context.actor_label} · ${display === "dev" ? "Developer" : "Secure"}`}
                variant="secondary"
                disabled={!runtime.active}
                onPress={() =>
                  onOpen({ nodeId, contextId: context.context_id, display })
                }
              />
            );
          }),
        )}
      {!query.error &&
        query.data &&
        !query.data.rows.length &&
        !query.data.contexts.length &&
        (conversationId ? (
          <Label muted>No desktop is linked to this conversation.</Label>
        ) : (
          <ActionButton
            compact
            label="View desktop"
            variant="secondary"
            disabled={!runtime.active}
            onPress={() => onOpen({ nodeId })}
          />
        ))}
      <Label muted>
        Viewing leaves the task running. Desktop availability is checked when
        opened.
      </Label>
    </>
  );
}
