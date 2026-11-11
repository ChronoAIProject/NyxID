import { useParams, useSearch } from "@tanstack/react-router";
import { CmaAgentView } from "@/components/assistant/cma-agent-view";

export function AssistantCmaAgentPage() {
  const { agentId } = useParams({ strict: false }) as { agentId: string };
  const search = useSearch({ strict: false }) as {
    response?: string;
    tab?: "desktop";
  };
  return (
    <CmaAgentView
      key={`${agentId}:${search.response ?? ""}`}
      agentId={agentId}
      responseId={search.response}
      initialTab={search.tab}
    />
  );
}
