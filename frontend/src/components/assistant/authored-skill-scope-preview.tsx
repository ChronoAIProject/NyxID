import { useQuery } from "@tanstack/react-query";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";
import { useAuthStore } from "@/stores/auth-store";

/** Read-only scope content from the encrypted proposal, never the receipt row. */
export function AuthoredSkillScopePreview({ agentId, proposalId }: {
  readonly agentId: string;
  readonly proposalId: string;
}) {
  const owner = useAuthStore((state) => state.user?.id);
  const preview = useQuery({
    queryKey: ["authored-skill-scope", owner, agentId, proposalId],
    queryFn: async () => authoredSkillPreviewSchema.parse(await assistantJson(
      `/assistant/nyxagent/agents/${encodeURIComponent(agentId)}/learning/proposals/${encodeURIComponent(proposalId)}`,
    )),
    enabled: Boolean(owner),
    retry: false,
    throwOnError: false,
  });
  if (preview.isError || preview.data?.scope_unavailable) {
    return <p className="text-11 text-muted-foreground">Scope unavailable.</p>;
  }
  const scope = preview.data?.scope;
  if (!scope?.omissions.length) return null;
  return (
    <div className="space-y-1 text-11 text-muted-foreground">
      <p>Declared omissions</p>
      <ul aria-label="Declared omissions" className="list-disc pl-5">
        {scope.omissions.map((omission, index) => <li key={index}>{omission}</li>)}
      </ul>
    </div>
  );
}
