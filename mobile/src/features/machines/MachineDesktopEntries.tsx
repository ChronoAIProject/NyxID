import { useAppLanguage } from "../../i18n/LanguageProvider";
import { translateApp } from "../../i18n/appTranslation";
import { useQuery } from "@tanstack/react-query";
import { z } from "zod";
import {
  SettingsGroup,
  SettingsRow,
} from "../../components/settings/SettingsList";
import { SkeletonRows } from "../../components/Skeleton";
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
  useAppLanguage();
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
    display === "dev" ? translateApp("Developer") : translateApp("Secure");
  if (query.error)
    return <Notice tone="error" message={machineError(query.error)} />;
  if (query.isPending)
    return (
      <SkeletonRows
        rows={1}
        icon={false}
        detail={false}
        title
        accessibilityLabel={translateApp("Checking desktop sessions")}
      />
    );
  return (
    <SettingsGroup
      title={translateApp("Desktop")}
      inset={false}
      footer={translateApp(
        "Viewing leaves the task running. Desktop availability is checked when opened.",
      )}
    >
      {query.data?.rows.map((row) => (
        <SettingsRow
          key={row.session_id}
          label={translateApp("{{displayName}} desktop", {
            displayName: displayName(row.display),
          })}
          value={row.status}
          accessibilityLabel={translateApp(
            "View desktop · {{displayName}} · {{code}}",
            { displayName: displayName(row.display), code: row.status },
          )}
          accessibilityHint={translateApp("Opens the live desktop viewer")}
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
                detail={translateApp("{{label}} · {{displayName}} desktop", {
                  label: context.actor_label,
                  displayName: displayName(display),
                })}
                accessibilityLabel={translateApp(
                  "View desktop · {{agentName}} · {{label}} · {{displayName}}",
                  {
                    agentName: context.agent_name,
                    label: context.actor_label,
                    displayName: displayName(display),
                  },
                )}
                accessibilityHint={translateApp(
                  "Opens the live desktop viewer",
                )}
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
          <SettingsRow label={translateApp("No desktop is linked to this conversation.")}
          />
        ) : (
          <SettingsRow
            label={translateApp("View Desktop")}
            accessibilityHint={translateApp("Opens the live desktop viewer")}
            disabled={!runtime.active}
            onPress={() => onOpen({ nodeId })}
          />
        ))}
    </SettingsGroup>
  );
}
