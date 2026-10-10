import { useState } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
import { authoredSkillPreviewSchema } from "@/schemas/agent-skills";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { publicationFailureText } from "@/lib/assistant/skill-publication-copy";

// A `publishing` row whose lease expired was interrupted (for example the
// request was cancelled). The server offers publish, retry, deny and discard
// only when nothing reached Ornn; otherwise the outcome is unknown and only a
// read-only check remains.
const NOTHING_SENT = ["publish", "retry", "deny", "discard"];

function interruptedCopy(actions: readonly string[]): string {
  if (actions.some((action) => NOTHING_SENT.includes(action))) {
    return "The last attempt was interrupted before anything was sent to Ornn.";
  }
  return actions.includes("check")
    ? "The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version; Check again only reads, it never writes again."
    : "The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version.";
}

export function AuthoredSkillCard({ acknowledgement: card, deciding, onDecision }: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly deciding: boolean;
  readonly onDecision: (choice: "allow" | "deny") => Promise<unknown>;
}) {
  const reference = card.authored_skill!;
  const owner = useAuthStore((s) => s.user?.id);
  const queryClient = useQueryClient();
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const preview = useQuery({
    queryKey: ["authored-skill-review", owner, reference.proposal_id, card.id],
    queryFn: async () => authoredSkillPreviewSchema.parse(await assistantJson(
      `/assistant/nyxagent/agents/${encodeURIComponent(reference.agent_id)}/learning/proposals/${encodeURIComponent(reference.proposal_id)}?acknowledgement_id=${encodeURIComponent(card.id)}`,
    )),
    enabled: Boolean(owner),
    refetchInterval: (query) => query.state.data?.status === "publishing" && query.state.data?.lease_live
      ? Math.min(3000 * 2 ** Math.min(query.state.dataUpdateCount, 3), 24000) : false,
    retry: false,
    throwOnError: false,
  });
  const data = preview.data;
  const actions = data?.actions ?? [];
  const path = `/assistant/nyxagent/agents/${encodeURIComponent(reference.agent_id)}/learning/proposals/${encodeURIComponent(reference.proposal_id)}`;
  async function refresh() {
    await Promise.all([
      preview.refetch(),
      queryClient.invalidateQueries({ queryKey: [...nyxBotQueryKeys.root(owner), "history"] }),
    ]);
  }
  async function decide(choice: "allow" | "deny") {
    setError(undefined);
    try { await onDecision(choice); }
    catch { setError("Publication did not finish. The current state is shown below."); }
    finally { await refresh(); }
  }
  async function postAction(action: "approve" | "reject" | "reprepare") {
    setError(undefined);
    setNotice(undefined);
    try {
      const result = await assistantJson<{ card_pending?: boolean } | undefined>(`${path}/${action}`, {
        method: "POST",
        body: action === "reprepare"
          ? { acknowledgement_id: card.id }
          : action === "approve"
            ? { revision: reference.revision, agent_skills_revision: data?.current_skills_revision, renewal_of: card.id }
            : { revision: reference.revision, reason: "rejected" },
      });
      if (action === "approve") setNotice("A new confirmation was posted in this conversation.");
      if (action === "reprepare") {
        setNotice(result?.card_pending
          ? "The package was rebuilt, but its review card was not posted. Use Show updated draft to post it."
          : "The package was rebuilt from the source skill. Review the new confirmation posted in this conversation.");
      }
    } catch {
      setError(action === "reprepare"
        ? "The package could not be rebuilt. The current state is shown below."
        : "The request could not be completed. Reload the current state and try again.");
    }
    finally { await refresh(); }
  }
  const messages = <>
    {error ? <p role="alert" className="text-[12px] text-destructive">{error}</p> : null}
    {notice ? <p role="status" className="text-[12px] text-muted-foreground">{notice}</p> : null}
  </>;
  const buttons = actions.length > 0 ? <div className="flex justify-end gap-2">
    {actions.includes("deny") ? <Button variant="outline" disabled={deciding} onClick={() => void decide("deny")}>Deny</Button> : null}
    {actions.includes("dismiss_card") ? <Button variant="outline" disabled={deciding} onClick={() => void decide("deny")}>Dismiss confirmation</Button> : null}
    {actions.includes("discard") ? <Button variant="outline" disabled={deciding} onClick={() => void postAction("reject")}>Discard</Button> : null}
    {actions.includes("renew") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("approve")}>Request a new confirmation</Button> : null}
    {actions.includes("publish") ? <Button variant="primary" disabled={deciding} onClick={() => void decide("allow")}>Allow and publish</Button> : null}
    {actions.includes("retry") ? <Button variant="primary" disabled={deciding} onClick={() => void decide("allow")}>Retry publication</Button> : null}
    {actions.includes("check") ? <Button variant="primary" disabled={deciding} onClick={() => void decide("allow")}>Check again</Button> : null}
    {actions.includes("reprepare") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("reprepare")}>Review updated package</Button> : null}
    {actions.includes("show_updated_draft") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("reprepare")}>Show updated draft</Button> : null}
  </div> : null;
  if (data?.state && data.state !== "active") {
    const status = <p role="status" className="ml-[30px] text-[11px] text-muted-foreground">
      {data.state === "pinned" ? "Skill published and attached" : data.state === "discarded" ? "Skill draft discarded" : data.state === "invalidated" ? "Skill draft is no longer available" : data.state === "dismissed" ? "Confirmation dismissed; the publication remains available." : "This confirmation no longer matches the draft."}
    </p>;
    if (!buttons && !error && !notice) return status;
    // An older card of a rebuilt draft can still raise the new package's card.
    return <div className="space-y-2">
      {status}
      <div className="ml-[30px] space-y-2">
        {actions.includes("show_updated_draft") ? <p className="text-[12px] text-muted-foreground">A rebuilt package for this draft is waiting for its review card.</p> : null}
        {messages}
        {buttons}
      </div>
    </div>;
  }
  return <section aria-label="Review skill draft" className="ml-[30px] space-y-3 rounded-xl border border-border/50 bg-card p-4">
    <p className="text-[13px] font-medium">Review skill draft{data ? `: ${data.name} · ${data.version}` : ""}</p>
    {data ? <p className="text-[12px]">Attach to {data.agent_name}</p> : null}
    <p className="text-[12px] text-muted-foreground">Allow publishes these files privately to Ornn under your identity and attaches this exact version to the agent. Skill instructions never grant permissions.</p>
    {data?.base_scripts_not_copied ? <p className="text-[12px] text-muted-foreground">Scripts and executable files from the source skill are not copied into this draft.</p> : null}
    {data?.files.map((file) => <div key={file.path}>
      <p className="text-[12px] font-medium">{file.path}</p>
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-3 text-[11px]">{file.content}</pre>
    </div>)}
    {data?.status === "publishing" ? <p role="status" className="text-[12px] text-muted-foreground">{data.lease_live ? "Publication is in progress." : interruptedCopy(actions)}</p> : null}
    {preview.isLoading ? <p role="status">Loading complete draft…</p> : null}
    {preview.isError ? <p role="alert" className="text-[12px] text-destructive">Draft unavailable or changed. Reload and review it again.</p> : null}
    {data?.failure_code ? <p role="alert" className="text-[12px] text-destructive">{publicationFailureText(data.failure_code, data.status)}</p> : null}
    {messages}
    {buttons}
  </section>;
}
