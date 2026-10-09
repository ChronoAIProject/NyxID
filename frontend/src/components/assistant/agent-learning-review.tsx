import { useState } from "react";
import { Link } from "@tanstack/react-router";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useAgentLearningReview } from "@/hooks/use-agent-learning-review";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { publicationFailureText } from "@/lib/assistant/skill-publication-copy";

function requestFailure(cause: unknown): string {
  return cause instanceof Error && cause.message
    ? cause.message
    : "The request could not be completed. Check the proposal and try again.";
}

export function AgentLearningReview({
  agentId,
  readOnly = false,
}: {
  readonly agentId: string;
  readonly readOnly?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.AGENT_LEARNING);
  const review = useAgentLearningReview(agentId, enabled);
  const [open, setOpen] = useState<string>();
  const [drafts, setDrafts] = useState<Record<string, string>>({});
  const [cards, setCards] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [threshold, setThreshold] = useState(15);

  if (!enabled || (readOnly && !review.data)) {
    return null;
  }

  const learningEnabled = review.status.data?.enabled ?? false;
  const proposals = review.data?.proposals ?? [];

  return (
    <section aria-label="Learning" className="space-y-3">
      <div className="space-y-1">
        <h3 className="text-13 font-semibold text-foreground">
          Learning proposals
        </h3>
        <p className="text-12 text-muted-foreground">
          Completed work can produce private, untrusted Ornn skill drafts.
          Publishing and attaching always requires your confirmation.
        </p>
      </div>
      {review.error ? (
        <p role="alert" className="text-12 text-destructive">
          Automatic learning is unavailable.
        </p>
      ) : null}

      {!readOnly ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/50 p-3">
          <label className="flex items-center gap-2 text-12">
            <Switch
              checked={learningEnabled}
              disabled={review.configure.isPending}
              onCheckedChange={(checked) =>
                review.configure.mutate({
                  enabled: checked,
                  threshold: Math.min(50, Math.max(1, threshold)),
                })
              }
            />
            Enable automatic learning
          </label>
          <label className="flex items-center gap-2 text-12">
            <span>Threshold</span>
            <Input
              aria-label="Learning threshold"
              className="w-16"
              type="number"
              min={1}
              max={50}
              value={threshold}
              onChange={(event) =>
                setThreshold(Number(event.target.value) || 15)
              }
              onBlur={() => {
                if (learningEnabled) {
                  review.configure.mutate({
                    enabled: true,
                    threshold: Math.min(50, Math.max(1, threshold)),
                  });
                }
              }}
            />
          </label>
        </div>
      ) : null}

      {proposals.length === 0 ? (
        <p className="text-12 text-text-tertiary">
          No pending proposals.
        </p>
      ) : null}

      {proposals.map((proposal) => {
        const draft = proposal.draft;
        const draftText = drafts[proposal.id] ?? draft?.skill_md ?? "";
        const isOpen = open === proposal.id;
        const card = cards[proposal.id];
        const checkOnly = proposal.evidence_available === false;
        // A verified version NyxID refused to attach is settled; the server
        // refuses to check it again.
        const settled =
          proposal.status === "published_unpinned" &&
          (proposal.failure_code === "evidence_unavailable" ||
            proposal.failure_code === "base_changed");
        return (
          <article
            key={proposal.id}
            className="space-y-3 rounded-lg border border-border/50 p-3"
          >
            <div className="flex items-center justify-between gap-2">
              <p className="text-12 font-medium">
                {draft?.name ?? "Learning proposal"}
              </p>
              <Badge variant="secondary">{proposal.status}</Badge>
            </div>
            <p className="text-12 text-muted-foreground">
              {draft?.description}
            </p>
            <p className="text-11 text-text-tertiary">
              {proposal.evidence_count} evidence items ·{" "}
              {proposal.body_bytes.toLocaleString()} bytes
            </p>
            {proposal.failure_code ? (
              <p role="alert" className="text-11 text-destructive">
                {publicationFailureText(proposal.failure_code, proposal.status)}
              </p>
            ) : null}
            {errors[proposal.id] ? (
              <p role="alert" className="text-11 text-destructive">
                {errors[proposal.id]}
              </p>
            ) : null}
            {checkOnly ? (
              <p role="status" className="text-11 text-warning">
                Learning evidence or consent is no longer available. NyxID
                can only check whether this publication reached Ornn; it will
                not attach it to the agent.
              </p>
            ) : null}

            {isOpen && draft ? (
              <div className="space-y-2">
                <p className="text-11 text-text-tertiary">
                  Generated guidance is untrusted and is shown as plain text.
                </p>
                <Textarea
                  aria-label="Skill draft"
                  className="min-h-40 font-mono text-11"
                  value={draftText}
                  readOnly={readOnly}
                  onChange={(event) =>
                    setDrafts((current) => ({
                      ...current,
                      [proposal.id]: event.target.value,
                    }))
                  }
                />
                <pre className="max-h-32 overflow-auto whitespace-pre-wrap rounded-md bg-muted/30 p-2 text-11">
                  {draft.safety_notes}
                </pre>
                {!readOnly &&
                drafts[proposal.id] !== undefined &&
                drafts[proposal.id] !== draft.skill_md ? (
                  <Button
                    size="sm"
                    variant="secondary"
                    disabled={review.edit.isPending}
                    onClick={() =>
                      review.edit.mutate({
                        proposalId: proposal.id,
                        draft: {
                          ...draft,
                          revision: proposal.revision,
                          skill_md: drafts[proposal.id],
                        },
                      })
                    }
                  >
                    Save draft
                  </Button>
                ) : null}
              </div>
            ) : null}

            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                onClick={() => setOpen(isOpen ? undefined : proposal.id)}
              >
                {isOpen ? "Hide draft" : "Review draft"}
              </Button>
              {proposal.source === "authored" ? (
                proposal.card_conversation_id ? (
                  <Link
                    to="/assistant"
                    search={{ c: proposal.card_conversation_id }}
                    className="text-11 text-primary hover:underline"
                  >
                    Open the confirmation card
                  </Link>
                ) : (
                  <p className="text-11 text-muted-foreground">
                    Confirm this authored skill from its conversation card.
                  </p>
                )
              ) : settled ? null : <Button
                size="sm"
                disabled={readOnly || review.approve.isPending}
                onClick={async () => {
                  setErrors((current) => ({ ...current, [proposal.id]: "" }));
                  try {
                    const result = await review.approve.mutateAsync({
                      proposalId: proposal.id,
                      acknowledgementId: card,
                    });
                    const acknowledgementId =
                      result.acknowledgement?.acknowledgement_id;
                    if (
                      result.status === "confirmation_required" &&
                      acknowledgementId
                    ) {
                      setCards((current) => ({
                        ...current,
                        [proposal.id]: acknowledgementId,
                      }));
                    }
                  } catch (cause) {
                    setErrors((current) => ({
                      ...current,
                      [proposal.id]: requestFailure(cause),
                    }));
                  }
                }}
              >
                {checkOnly
                  ? card
                    ? "Confirm check"
                    : "Check publication"
                  : card
                    ? "Confirm publish & attach"
                    : "Publish & request confirmation"}
              </Button>}
              {/* The server rejects only pending or failed drafts. A failed
                  publication that may have reached Ornn is refused, and the
                  reason appears in the error line above. */}
              {proposal.status === "pending" ||
              proposal.status === "publication_failed" ? (
                <Button
                  size="sm"
                  disabled={readOnly || review.reject.isPending}
                  onClick={async () => {
                    setErrors((current) => ({ ...current, [proposal.id]: "" }));
                    try {
                      await review.reject.mutateAsync({
                        proposalId: proposal.id,
                        revision: proposal.revision,
                      });
                    } catch (cause) {
                      setErrors((current) => ({
                        ...current,
                        [proposal.id]: requestFailure(cause),
                      }));
                    }
                  }}
                >
                  Reject
                </Button>
              ) : null}
            </div>
            {card && proposal.source !== "authored" && !checkOnly ? (
              <p className="text-11 text-warning">
                Confirming this exact card will publish the private skill and
                attach its pinned version. The card expires if the draft or
                agent changes.
              </p>
            ) : null}
          </article>
        );
      })}

      {!readOnly ? (
        <Button
          size="sm"
          variant="secondary"
          disabled={review.run.isPending}
          onClick={() => review.run.mutate()}
        >
          Run learning now
        </Button>
      ) : null}
    </section>
  );
}
