import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";

export function AuthoredSkillCard({ acknowledgement: card, deciding, onDecision }: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly deciding: boolean;
  readonly onDecision: (choice: "allow" | "deny") => Promise<unknown>;
}) {
  const reference = card.authored_skill!;
  const owner = useAuthStore((s) => s.user?.id);
  const [error, setError] = useState<string>();
  const terminal = card.status === "denied" || card.status === "expired";
  const preview = useQuery({
    queryKey: ["authored-skill-review", owner, reference.proposal_id, reference.revision],
    queryFn: async () => authoredSkillPreviewSchema.parse(await assistantJson(
      `/assistant/nyxagent/agents/${encodeURIComponent(reference.agent_id)}/learning/proposals/${encodeURIComponent(reference.proposal_id)}`,
    )),
    enabled: Boolean(owner) && !terminal,
    refetchInterval: (query) => query.state.data?.status === "publishing" ? 3000 : false,
    retry: false,
    throwOnError: false,
  });
  const data = preview.data;
  const pinned = data?.status === "pinned";
  const current = data?.id === reference.proposal_id && data.agent_id === reference.agent_id &&
    data.revision === reference.revision && data.skills_revision === reference.skills_revision &&
    data.current_skills_revision === reference.skills_revision && data.files.length > 0 &&
    ["pending", "publishing", "publication_failed", "published_unpinned"].includes(data.status);
  async function decide(choice: "allow" | "deny") {
    setError(undefined);
    try { await onDecision(choice); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Could not publish. Try again."); }
    finally { await preview.refetch(); }
  }
  if (terminal || pinned) return <p role="status" className="ml-[30px] text-[11px] text-muted-foreground">
    {pinned ? "Skill published and attached" : card.status === "denied" ? "Skill draft denied" : "Skill review expired — nothing published"}
  </p>;
  return <section aria-label="Review skill draft" className="ml-[30px] space-y-3 rounded-xl border border-border/50 bg-card p-4">
    <p className="text-[13px] font-medium">Review skill draft{data ? `: ${data.name} · ${data.version}` : ""}</p>
    {data ? <p className="text-[12px]">Attach to {data.agent_name}</p> : null}
    <p className="text-[12px] text-muted-foreground">Allow publishes these files privately to Ornn under your identity and attaches this exact version to the agent. Skill instructions never grant permissions.</p>
    {data?.files.map((file) => <div key={file.path}>
      <p className="text-[12px] font-medium">{file.path}</p>
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-3 text-[11px]">{file.content}</pre>
    </div>)}
    {data?.status === "publishing" ? <p role="status" className="text-[12px] text-muted-foreground">Publication is in progress. Retry checks the same publication.</p> : null}
    {preview.isLoading ? <p role="status">Loading complete draft…</p> : null}
    {preview.isError || (data && !current) ? <p role="alert" className="text-[12px] text-destructive">Draft unavailable or changed. Reload and review it again.</p> : null}
    {error ? <p role="alert" className="text-[12px] text-destructive">{error}</p> : null}
    <div className="flex justify-end gap-2">
      <Button variant="outline" disabled={deciding || card.status !== "pending"} onClick={() => void decide("deny")}>Deny</Button>
      <Button variant="primary" disabled={deciding || !current} onClick={() => void decide("allow")}>
        {card.status === "pending" ? "Allow and publish" : "Retry publication"}
      </Button>
    </div>
  </section>;
}
