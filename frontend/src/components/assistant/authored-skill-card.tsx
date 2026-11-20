import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useId, useState } from "react";
import { STATUS_CARD_MAX_W } from "@/lib/assistant/banner";
import type { z } from "zod";
import { DecisionReceipt } from "./decision-receipt";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Button } from "@/components/ui/button";
import { assistantJson } from "@/lib/assistant/assistant-http";
import { useAuthStore } from "@/stores/auth-store";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
import { authoredSkillPreviewSchema, learningDraftEditable } from "@/schemas/agent-skills";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { publicationFailureText, publicationRuleText } from "@/lib/assistant/skill-publication-copy";
import { ProposalDraftEditor } from "./learning-draft-editor";

// A `publishing` row whose lease expired was interrupted (for example the
// request was cancelled). The server offers publish, retry, deny and discard
// only when nothing reached Ornn; otherwise the outcome is unknown and only a
// read-only check remains.
const NOTHING_SENT = ["publish", "retry", "deny", "discard"];

function interruptedCopy(actions: readonly string[]): string {
  if (actions.some((action) => NOTHING_SENT.includes(action))) {
    return translateUi("The last attempt was interrupted before anything was sent to Ornn.");
  }
  return actions.includes("check")
    ? translateUi("The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version; Check again only reads, it never writes again.")
    : translateUi("The last attempt was interrupted. NyxID cannot yet confirm whether Ornn published this version.");
}

type AuthoredPreview = z.infer<typeof authoredSkillPreviewSchema>;

function FailureDetails({ data }: { readonly data: AuthoredPreview }) {
  useUiTranslation();
  if (!data.failure_code) return null;
  return (
    <>
      <p role="alert" className="text-14 text-destructive">
        {publicationFailureText(data.failure_code, data.status, "authored")}
      </p>
      {data.failure_rules.length ? (
        <ul className="list-disc pl-5 text-13 text-muted-foreground">
          {data.failure_rules.map((rule) => <li key={rule}>{publicationRuleText(rule)}</li>)}
        </ul>
      ) : data.failure_details_unavailable ? (
        <p className="text-13 text-muted-foreground">{translateUi("Format details unavailable.")}</p>
      ) : null}
    </>
  );
}

function ScopeSection({ data, id, accepted, onAccepted }: {
  readonly data: AuthoredPreview;
  readonly id: string;
  readonly accepted: boolean;
  readonly onAccepted: (accepted: boolean) => void;
}) {
  useUiTranslation();
  if (data.scope_unavailable) {
    return (
      <p role="alert" className="text-13 text-muted-foreground">{translateUi("Scope unavailable. Reload the draft before allowing publication.")}</p>
    );
  }
  if (!data.scope) return null;
  const { status, omissions } = data.scope;
  return (
    <>
      {omissions.length ? (
        <ul aria-label={translateUi("Declared omissions")} className="list-disc pl-5 text-13 text-muted-foreground">
          {omissions.map((text, index) => <li key={index}>{text}</li>)}
        </ul>
      ) : null}
      {status === "reduced" ? (
        <div className="flex items-start gap-2">
          <Checkbox
            id={id}
            checked={accepted}
            onCheckedChange={(checked) => onAccepted(checked === true)}
          />
          <Label htmlFor={id}>{translateUi("I accept this reduced scope ({{count}} declared omissions).", { count: omissions.length })}
          </Label>
        </div>
      ) : (
        <p className="text-13 text-muted-foreground">{translateUi("The drafting agent declares this draft covers everything requested. The read coverage shown here is what NyxID observed.")}</p>
      )}
    </>
  );
}

function coverageText(upload: AuthoredPreview["coverage"][number]): string {
  if (upload.expired) return translateUi("expired");
  if (upload.unknown) return translateUi("read coverage unavailable");
  if (upload.reads === 0) return translateUi("not read");
  if (upload.lower_bound) return translateUi("at least {{percent}}%", { percent: upload.percent });
  if (upload.percent === 100) return translateUi("read completely");
  return translateUi("read {{percent}}%", { percent: upload.percent });
}

function CoverageList({ data }: { readonly data: AuthoredPreview }) {
  useUiTranslation();
  return (
    <>
      {data.coverage.length ? (
        <div className="space-y-1 text-13 text-muted-foreground">
          <p>{translateUi("Observed reads in the drafting conversation")}</p>
          <ul>
            {data.coverage.map((upload) => (
              <li key={upload.id}>
                {upload.name}: {coverageText(upload)}
                {upload.since_reset_percent != null ? (
                  <>; {upload.lower_bound
                    ? translateUi("at least {{percent}}% since the last context reset", { percent: upload.since_reset_percent })
                    : translateUi("read {{percent}}% since the last context reset", { percent: upload.since_reset_percent })}</>
                ) : null}
              </li>
            ))}
          </ul>
        </div>
      ) : null}
      {data.coverage_omitted ? (
        <p className="text-13 text-muted-foreground">
          {translateUi("{{uploads}} earlier uploads are not shown.", { uploads: data.coverage_omitted })}</p>
      ) : null}
    </>
  );
}

export const SAVED_EDIT_NOTICE =
  "Draft saved. A new review card for this version was posted in this conversation; confirm it there.";

export function AuthoredSkillCard({ acknowledgement: card, confirmationsBypassed = false, deciding, onDecision }: {
  readonly acknowledgement: NyxAgentAcknowledgement;
  readonly confirmationsBypassed?: boolean;
  readonly deciding: boolean;
  readonly onDecision: (choice: "allow" | "deny", acceptReducedScope?: boolean) => Promise<unknown>;
}) {
  useUiTranslation();
  const reference = card.authored_skill!;
  const owner = useAuthStore((s) => s.user?.id);
  const queryClient = useQueryClient();
  const [error, setError] = useState<string>();
  const [notice, setNotice] = useState<string>();
  const [acceptReducedScope, setAcceptReducedScope] = useState(false);
  const scopeId = useId();
  const [editing, setEditing] = useState(false);
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
  useEffect(() => {
    setAcceptReducedScope(false);
  }, [data?.revision]);
  const scopePending = data?.scope_unavailable
    || (data?.scope?.status === "reduced" && !acceptReducedScope);
  const path = `/assistant/nyxagent/agents/${encodeURIComponent(reference.agent_id)}/learning/proposals/${encodeURIComponent(reference.proposal_id)}`;
  async function refresh() {
    await Promise.all([
      preview.refetch(),
      queryClient.invalidateQueries({ queryKey: [...nyxBotQueryKeys.root(owner), "history"] }),
    ]);
  }
  async function decide(choice: "allow" | "deny") {
    setError(undefined);
    try { await (acceptReducedScope ? onDecision(choice, true) : onDecision(choice)); }
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
    {error ? <p role="alert" className="text-14 text-destructive">{translateUi(error)}</p> : null}
    {notice ? <p role="status" className="text-14 text-muted-foreground">{translateUi(notice)}</p> : null}
  </>;
  const buttons = actions.length > 0 ? <div className="flex justify-end gap-2">
    {actions.includes("deny") ? <Button variant="outline" disabled={deciding} onClick={() => void decide("deny")}>{translateUi("Deny")}</Button> : null}
    {actions.includes("dismiss_card") ? <Button variant="outline" disabled={deciding} onClick={() => void decide("deny")}>{translateUi("Dismiss confirmation")}</Button> : null}
    {actions.includes("discard") ? <Button variant="outline" disabled={deciding} onClick={() => void postAction("reject")}>{translateUi("Discard")}</Button> : null}
    {actions.includes("renew") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("approve")}>{translateUi("Request a new confirmation")}</Button> : null}
    {actions.includes("publish") ? <Button variant="primary" disabled={deciding || scopePending} onClick={() => void decide("allow")}>{translateUi("Allow and publish")}</Button> : null}
    {actions.includes("retry") ? <Button variant="primary" disabled={deciding || scopePending} onClick={() => void decide("allow")}>{translateUi("Retry publication")}</Button> : null}
    {actions.includes("check") ? <Button variant="primary" disabled={deciding || scopePending} onClick={() => void decide("allow")}>{translateUi("Check again")}</Button> : null}
    {actions.includes("reprepare") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("reprepare")}>{translateUi("Review updated package")}</Button> : null}
    {actions.includes("show_updated_draft") ? <Button variant="primary" disabled={deciding} onClick={() => void postAction("reprepare")}>{translateUi("Show updated draft")}</Button> : null}
  </div> : null;
  if (confirmationsBypassed) {
    return <DecisionReceipt acknowledgement={card} retired={card.status === "pending"}
      summary={data?.state === "pinned" ? translateUi("Skill {{name}} {{version}} published and attached to {{agent}}", {
        name: data.name, version: data.version, agent: data.agent_name,
      }) : undefined} />;
  }
  if (data?.state && data.state !== "active") {
    const stateSummary = data.state === "pinned" ? translateUi("Skill {{name}} {{version}} published and attached to {{agent}}", {
        name: data.name, version: data.version, agent: data.agent_name,
      })
      : data.state === "discarded" ? translateUi("Skill draft discarded")
      : data.state === "invalidated" ? translateUi("Skill draft is no longer available")
      : data.state === "dismissed" ? translateUi("Confirmation dismissed; the publication remains available.")
      : translateUi("This confirmation no longer matches the draft.");
    const status = <DecisionReceipt acknowledgement={data.state === "pinned" ? { ...card, status: "allowed", decided_by: "user" } : card}
      summary={stateSummary} />;
    if (!buttons && !error && !notice) return status;
    // An older card of a rebuilt draft can still raise the new package's card.
    return <div className="space-y-2">
      {status}
      <div className="ml-[30px] space-y-2">
        {actions.includes("show_updated_draft") ? <p className="text-14 text-muted-foreground">{translateUi("A rebuilt package for this draft is waiting for its review card.")}</p> : null}
        {messages}
        {buttons}
      </div>
    </div>;
  }
  if (!data && card.status !== "pending") return <DecisionReceipt acknowledgement={card} />;
  return <section aria-label={translateUi("Review skill draft")} className={`${STATUS_CARD_MAX_W} ml-[30px] space-y-3 rounded-xl border border-border/50 bg-card p-4`}>
    <p className="type-card-title">{data
      ? translateUi("Review skill draft: {{name}} · {{version}}", { name: data.name, version: data.version })
      : translateUi("Review skill draft")}</p>
    {data ? <p className="text-14">{translateUi("Attach to {{agent}}", { agent: data.agent_name })}</p> : null}
    <p className="text-14 text-muted-foreground">{translateUi("Allow publishes these files privately to Ornn under your identity and attaches this exact version to the agent. Skill instructions never grant permissions.")}</p>
    {data?.base_scripts_not_copied ? <p className="text-14 text-muted-foreground">{translateUi("Scripts and executable files from the source skill are not copied into this draft.")}</p> : null}
    {data?.files.map((file) => <div key={file.path}>
      <p className="type-row-title">{file.path}</p>
      <pre className="max-h-80 overflow-auto whitespace-pre-wrap break-words rounded-lg bg-muted p-3 text-14">{file.content}</pre>
    </div>)}
    {data && learningDraftEditable(data.status) ? <div className="space-y-2">
      <Button variant="outline" size="sm" aria-expanded={editing} disabled={deciding} onClick={() => setEditing(!editing)}>
        {editing ? translateUi("Close the editor") : translateUi("Edit before publishing")}
      </Button>
      {editing ? <>
        <p className="text-14 text-muted-foreground">{translateUi("Saving changes this draft and posts a new review card for the saved version in this conversation. Publish from that new card.")}</p>
        <ProposalDraftEditor agentId={reference.agent_id} proposalId={reference.proposal_id} showCardLink={false} onSaved={() => {
          setEditing(false);
          setNotice(SAVED_EDIT_NOTICE);
          void refresh();
        }} />
      </> : null}
    </div> : null}
    {data?.status === "publishing" ? <p role="status" className="text-14 text-muted-foreground">{data.lease_live ? translateUi("Publication is in progress.") : interruptedCopy(actions)}</p> : null}
    {preview.isLoading ? <p role="status">{translateUi("Loading complete draft…")}</p> : null}
    {preview.isError ? <p role="alert" className="text-14 text-destructive">{translateUi("Draft unavailable or changed. Reload and review it again.")}</p> : null}
    {data ? (
      <>
        <FailureDetails data={data} />
        <ScopeSection
          data={data}
          id={scopeId}
          accepted={acceptReducedScope}
          onAccepted={setAcceptReducedScope}
        />
        <CoverageList data={data} />
      </>
    ) : null}
    {messages}
    {buttons}
  </section>;
}
