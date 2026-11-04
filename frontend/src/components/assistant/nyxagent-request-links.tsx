import type { AssistantAgentRequest } from "@/schemas/assistant-nyxagent";

/** The source owns the card; a routed notice does not move it into NyxBot. */
export function NyxAgentRequestLinks({ requests, conversationId }: {
  readonly requests: readonly AssistantAgentRequest[];
  readonly conversationId?: string;
}) {
  const elsewhere = requests.filter((row) => row.review_path && row.conversation_id !== conversationId);
  if (!elsewhere.length) return null;
  return <details className="rounded-lg border border-border/50 bg-card px-3 py-2 text-12">
    <summary className="cursor-pointer text-muted-foreground">Specialist requests in other conversations ({elsewhere.length})</summary>
    <ul className="mt-2 space-y-1">
      {elsewhere.map((row) => <li key={row.request_id}>
        <a className="underline" href={row.review_path}>{row.agent}: {row.summary}</a>
        {row.status && row.status !== "pending" ? <span className="ml-2 text-muted-foreground">{row.status}</span> : null}
      </li>)}
    </ul>
  </details>;
}
