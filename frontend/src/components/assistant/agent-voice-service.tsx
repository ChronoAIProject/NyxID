import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { z } from "zod";
import { useContext } from "react";
import { Link } from "@tanstack/react-router";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { ErrorBanner } from "@/components/shared/error-banner";
import { useFeature } from "@/hooks/use-feature-flag";
import { useAgentOwnerMutation } from "@/hooks/use-agent-owner-mutation";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { agentTitle } from "@/lib/assistant/nyxbot-labels";
import { useAuthStore } from "@/stores/auth-store";
import type { AssistantAgent } from "@/schemas/assistant-nyxagent";
import { GroupedList, GroupedRow, GroupedDisclosure } from "@/components/ui/grouped-list";
import { DrawerNavigationContext } from "@/lib/drawer-search";

const response = z.object({
  service: z
    .object({
      id: z.string().uuid(),
      slug: z.string(),
      agent_id: z.string().uuid(),
    })
    .nullable(),
});

export function AgentVoiceService({
  agent,
}: {
  readonly agent: AssistantAgent;
}) {
  useUiTranslation();
  const enabled = useFeature(FEATURE_FLAG.VOICE_AGENT_KEY);
  const owner = useAuthStore((s) => s.user?.id);
  if (
    !enabled ||
    !owner ||
    agent.owner_kind === "org" ||
    agent.share_role ||
    agent.status === "destroyed"
  )
    return null;
  return (
    <VoiceConnection key={`${owner}:${agent.id}`} agent={agent} owner={owner} />
  );
}

function VoiceConnection({
  agent,
  owner,
}: {
  readonly agent: AssistantAgent;
  readonly owner: string;
}) {
  useUiTranslation();
  const navigate = useContext(DrawerNavigationContext);
  const client = useQueryClient();
  const queryKey = ["assistant", "voice-service", owner, agent.id];
  const path = `/assistant/nyxagent/agents/${encodeURIComponent(agent.id)}/voice-service`;
  const query = useQuery({
    queryKey,
    queryFn: async () => response.parse(await assistantJson(path)),
    retry: false,
  });
  const publish = useAgentOwnerMutation({
    mutationFn: async (_: void, guard) => {
      guard();
      return response.parse(
        await assistantJson(path, { method: "PUT", authorityGuard: guard }),
      );
    },
    onSettled: async () => {
      await Promise.all([
        client.invalidateQueries({ queryKey }),
        client.invalidateQueries({ queryKey: ["keys"] }),
        client.invalidateQueries({ queryKey: ["user-services"] }),
      ]);
    },
  });
  const error = query.error || (!query.data?.service ? publish.error : null);
  return (
    <section aria-label={translateUi("Device voice")} className="space-y-3">
      <GroupedList title={translateUi("Device voice")} search={{ id: "device-voice", title: translateUi("Device voice"), breadcrumb: "Access", keywords: "phone call instructions setup Agent Key" }} footer={<>
        <p>{translateUi("Optional device integration for {{agent}} using its existing access, history and billing. Ordinary chat does not require this setup.", { agent: agentTitle(agent) })}</p>
        <p>{agent.kind === "nyxbot" ? translateUi("This NyxBot has full account and service access.") : translateUi("Other Bots require their own connections.")}</p>
        <p>{translateUi("Creating the connection grants no Key explicitly. Keys set to all services include it under their existing policy.")}</p>
        <p>{translateUi("Setup and access changes apply immediately.")}</p>
      </>}>
      {error ? (
        <GroupedRow><ErrorBanner
          message={
            error instanceof Error
              ? error.message
              : translateUi("Could not load voice connection.")
          }
          onRetry={() => { publish.reset(); void query.refetch(); }}
        /></GroupedRow>
      ) : null}
      {query.data?.service ? (
        <>
          <GroupedRow title={translateUi("Voice connection created")} description={query.data.service.slug} />
          <GroupedRow title={translateUi("Check device access")} description={translateUi("The device’s Agent Key must permit this connection. All-services Keys can already include it; selected-service Keys may need an access change. A created connection does not confirm device readiness.")} accessory={
          <Button variant="outline" size="sm" asChild>
            <Link to="/api-keys">{translateUi("Manage Agent Key access")}</Link>
          </Button>
          } />
          <GroupedRow description={translateUi("Remove the connection from the Key to revoke access, or disable it in Services.")} />
        </>
      ) : (
        <GroupedRow><Button
          variant="outline"
          size="sm"
          disabled={
            query.isPending || Boolean(query.error) || publish.isPending
          }
          onClick={() => publish.mutate()}
        >
          {publish.isPending ? translateUi("Creating…") : translateUi("Create voice connection")}
        </Button></GroupedRow>
      )}
      {navigate ? <GroupedDisclosure onClick={() => void navigate("profile-voice-instructions")}>{translateUi("Edit call instructions")}</GroupedDisclosure> : null}
      </GroupedList>
    </section>
  );
}
