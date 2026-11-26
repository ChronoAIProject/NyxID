import { useCardTranslation } from "@/components/assistant/card-language";
import { useEffect, useRef, useState } from "react";
import { usePinnedDecisionCard } from "../decision-card-presentation";
import { DecisionCardActions } from "../decision-card-actions";
import { DecisionReview } from "../decision-review";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { api } from "@/lib/api-client";
import {
  executionProposalSchema,
  executionContextLabel,
  type ExecutionProposal,
} from "@/schemas/execution-proposal";
import type { ApprovalRequestItem } from "@/types/approvals";

const statusLabel: Record<string, string> = {
  pending: "Execution permission required",
  queued: "Permission saved · operation queued",
  preparing: "Permission saved · preparing operation",
  dispatched: "Permission saved · operation running",
  waiting: "Permission saved · separate review required",
  completed: "Operation completed",
  denied: "Permission denied · nothing sent",
  expired: "Permission request expired · nothing sent",
  cancelled: "Task stopped · nothing sent",
  not_sent: "Permission saved · operation not sent",
  unknown: "Operation outcome unknown · will not repeat",
  unsupported: "Open the current web app to review this permission",
};

export function ExecutionPermissionCard({
  requestId,
  proposal,
  disabled = false,
  onStateChange,
}: {
  readonly requestId: string;
  readonly proposal: ExecutionProposal;
  readonly disabled?: boolean;
  readonly onStateChange?: () => void;
}) {
  const { t: translateUi, language } = useCardTranslation();
  const pinned = usePinnedDecisionCard();
  const queryClient = useQueryClient();
  const [choiceId, setChoiceId] = useState("operation");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const submission = useRef<{ signature: string; key: string } | null>(null);
  const query = useQuery({
    queryKey: ["execution-permission-review", requestId],
    initialData: proposal,
    queryFn: async () => {
      const row = await api.get<ApprovalRequestItem>(
        `/approvals/requests/${requestId}`,
      );
      return executionProposalSchema.parse(row.execution_proposal);
    },
    refetchInterval: ({ state }) =>
      ["pending", "queued", "preparing", "dispatched", "waiting"].includes(
        state.data?.state ?? "",
      )
        ? 2000
        : false,
  });
  const current = query.data;
  const couldDecide = useRef(proposal.can_decide);
  useEffect(() => {
    if (couldDecide.current && !current.can_decide) onStateChange?.();
    couldDecide.current = current.can_decide;
  }, [current.can_decide, onStateChange]);
  const choice = current.choices.find((c) => c.id === choiceId);
  async function decide(approved: boolean) {
    if (busy || disabled || !current.can_decide || (approved && !choice))
      return;
    const signature = `${current.revision}:${approved ? choice?.digest : "deny"}`;
    if (submission.current?.signature !== signature)
      submission.current = { signature, key: crypto.randomUUID() };
    setBusy(true);
    setError(null);
    try {
      const row = await api.post<ApprovalRequestItem>(
        `/approvals/requests/${requestId}/execution-decision`,
        {
          approved,
          expected_revision: current.revision,
          choice_id: approved ? choice!.id : null,
          choice_digest: approved ? choice!.digest : null,
          idempotency_key: submission.current.key,
        },
      );
      queryClient.setQueryData(
        ["execution-permission-review", requestId],
        executionProposalSchema.parse(row.execution_proposal),
      );
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["approvals"] }),
        queryClient.invalidateQueries({ queryKey: ["collaboration-group"] }),
      ]);
    } catch (failure) {
      setError(
        failure instanceof Error
          ? failure.message
          : "The decision could not be saved. Check its status before retrying.",
      );
      void query.refetch();
    } finally {
      setBusy(false);
    }
  }
  const actions = current.can_decide && current.supported ? <div className="flex flex-wrap justify-end gap-2">
    <Button variant="outline" disabled={disabled || busy} onClick={() => void decide(false)}>{translateUi("Deny")}</Button>
    <Button disabled={disabled || busy || !choice} onClick={() => void decide(true)}>{translateUi("Allow")}{" "}{choice?.id === "operation" ? translateUi("this operation") : translateUi("selected permission")}</Button>
  </div> : null;
  const content = <>
      <p className="type-card-title">
        {translateUi(statusLabel[current.state] ?? "Permission status unavailable")}
      </p>
      {current.review && (
        <dl className="space-y-1 text-12">
          <div>
            <dt className="inline text-muted-foreground">{current.source ? translateUi("Source: ") : translateUi("Agent: ")}</dt>
            <dd className="inline">{current.source ?? current.review.agent}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Connection: ")}</dt>
            <dd className="inline">{current.review.connection}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Operation: ")}</dt>
            <dd className="inline break-all">{current.review.operation}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Audience: ")}</dt>
            <dd className="inline">{current.review.audience}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Scope: ")}</dt>
            <dd className="inline">{current.review.scope}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Current permission:")}{" "}
            </dt>
            <dd className="inline">{current.review.existing_permission}</dd>
          </div>
          <div>
            <dt className="inline text-muted-foreground">{translateUi("Adds: ")}</dt>
            <dd className="inline">{current.review.added_permission}</dd>
          </div>
          <div>
            <dt className="text-muted-foreground">{translateUi("Inputs")}</dt>
            <dd className={pinned ? "whitespace-pre-wrap break-all" : "max-h-48 overflow-auto whitespace-pre-wrap break-all"}>
              {current.review.inputs}
            </dd>
          </div>
        </dl>
      )}
      {current.can_decide && current.supported && (
        <>
          <fieldset className="space-y-2" disabled={disabled || busy}>
            <Label asChild>
              <legend className="mb-2">{translateUi("Allow for")}</legend>
            </Label>
            {current.choices.map((c) => (
              <Label key={c.id} className="flex items-start gap-2">
                <input
                  type="radio"
                  name={`permission-validity-${requestId}`}
                  value={c.id}
                  checked={choiceId === c.id}
                  onChange={() => setChoiceId(c.id)}
                />
                <span>
                  {c.label}
                  {c.valid_until && (
                    <span className="block text-muted-foreground">{translateUi("Until ")}{new Date(c.valid_until).toLocaleString(language)}
                    </span>
                  )}
                </span>
              </Label>
            ))}
          </fieldset>
          {choice?.context_scope && (
            <p className="text-12 text-muted-foreground">{translateUi("Applies to: ")}{translateUi(executionContextLabel(choice.context_scope) ?? "")}
            </p>
          )}
          {current.review?.reusable_unavailable && (
            <p className="text-12 text-muted-foreground">
              {current.review.reusable_unavailable}
            </p>
          )}
          <p className="text-12 text-muted-foreground">{translateUi("This saves the displayed permission and resumes the retained operation. Other required reviews still apply.")}</p>
        </>
      )}
      {current.state === "expired" && (
        <p className="text-12">
          {current.version === 2
            ? translateUi("This expired operation will not be sent. To request a new operation, use a new request context.")
            : translateUi("Return to the original conversation and explicitly request the task again. This expired operation will not be sent.")}
        </p>
      )}
      {error && (
        <p role="alert" className="text-12 text-destructive">
          {translateUi(error)}
        </p>
      )}
      {query.isError && (
        <p role="alert" className="text-12 text-destructive">{translateUi("The current status could not be loaded.")}</p>
      )}
      {current.result_text && (
        <details className="text-12">
          <summary className="cursor-pointer">{translateUi("Your private operation result")}</summary>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap break-all font-sans">{current.result_text}</pre>
        </details>
      )}
      {current.grant_id && (
        <a href="/approvals/grants" className="text-12 underline">{translateUi("Inspect or revoke the saved permission")}</a>
      )}
  </>;
  if (pinned && current.can_decide && current.supported) {
    return <DecisionReview title={translateUi("Execution permission required")} actions={actions} busy={disabled || busy} onDeny={() => void decide(false)}
      summary={<>
        {current.review ? <><p className="line-clamp-1 break-words">{current.review.connection}</p><p className="line-clamp-2 break-words">{current.review.operation}</p></> : null}
        <p>{translateUi("Review the exact inputs and permission scope before allowing.")}</p>
        {error ? <p role="alert" className="text-destructive">{error}</p> : null}
      </>}
    ><div className="space-y-3">{content}</div></DecisionReview>;
  }
  return <section lang={language} className={pinned ? "space-y-3 p-4" : "max-w-xl space-y-3 rounded-xl border border-border bg-card p-4"} aria-label={translateUi("Execution permission")}>
    {content}
    <DecisionCardActions>{actions}</DecisionCardActions>
  </section>;
}
