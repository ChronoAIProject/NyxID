import { z } from "zod";
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
  const error = query.error || publish.error;
  return (
    <section aria-label="Device voice" className="space-y-3">
      <h3 className="text-13 font-semibold text-foreground">Device voice</h3>
      <p className="text-12 text-muted-foreground">
        Create a voice connection for {agentTitle(agent)}, then allow this
        connection on your device’s Agent Key. It permits conversations with
        this Bot using its existing access, history and billing. Actions requiring
        owner confirmation are reviewed on their NyxID cards.
        {agent.kind === "nyxbot"
          ? " This NyxBot has full account and service access."
          : " Other Bots require their own connections."}
      </p>
      <p className="text-12 text-muted-foreground">
        Creating the connection grants no Key explicitly. Keys set to all
        services include it under their existing policy.
      </p>
      {error ? (
        <ErrorBanner
          message={
            error instanceof Error
              ? error.message
              : "Could not load voice connection."
          }
        />
      ) : null}
      {query.data?.service ? (
        <>
          <p className="break-all text-12">
            Voice connection: <code>{query.data.service.slug}</code>
          </p>
          <Button variant="outline" size="sm" asChild>
            <Link to="/api-keys">Manage Agent Key access</Link>
          </Button>
          <p className="text-12 text-muted-foreground">
            Remove the connection from the Key to revoke access, or disable it
            in Services.
          </p>
        </>
      ) : (
        <Button
          variant="outline"
          size="sm"
          disabled={
            query.isPending || Boolean(query.error) || publish.isPending
          }
          onClick={() => publish.mutate()}
        >
          {publish.isPending ? "Creating…" : "Create voice connection"}
        </Button>
      )}
    </section>
  );
}
