import { useState } from "react";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { useAgentLearningReview } from "@/hooks/use-agent-learning-review";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";

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
  const [threshold, setThreshold] = useState(15);

  if (!enabled || (readOnly && !review.data)) {
    return null;
  }

  const learningEnabled = review.status.data?.enabled ?? false;
  const proposals = review.data?.proposals ?? [];

  return (
    <section aria-label="Learning" className="space-y-3">
      <div className="space-y-1">
        <h3 className="text-[13px] font-semibold text-foreground">
          Learning proposals
        </h3>
        <p className="text-[12px] text-muted-foreground">
          Completed work can produce private, untrusted Ornn skill drafts.
          Publishing and attaching always requires your confirmation.
        </p>
      </div>
      {review.error ? (
        <p role="alert" className="text-[12px] text-destructive">
          Automatic learning is unavailable.
        </p>
      ) : null}

      {!readOnly ? (
        <div className="flex flex-wrap items-center justify-between gap-3 rounded-lg border border-border/50 p-3">
          <label className="flex items-center gap-2 text-[12px]">
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
          <label className="flex items-center gap-2 text-[12px]">
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
        <p className="text-[12px] text-text-tertiary">
          No pending proposals.
        </p>
      ) : null}

      {proposals.map((proposal) => {
        const draft = proposal.draft;
        const draftText = drafts[proposal.id] ?? draft?.skill_md ?? "";
        const isOpen = open === proposal.id;
        const card = cards[proposal.id];
        return (
          <article
            key={proposal.id}
            className="space-y-3 rounded-lg border border-border/50 p-3"
          >
            <div className="flex items-center justify-between gap-2">
              <p className="text-[12px] font-medium">
                {draft?.name ?? "Learning proposal"}
              </p>
              <Badge variant="secondary">{proposal.status}</Badge>
            </div>
            <p className="text-[12px] text-muted-foreground">
              {draft?.description}
            </p>
            <p className="text-[11px] text-text-tertiary">
              {proposal.evidence_count} evidence items ·{" "}
              {proposal.body_bytes.toLocaleString()} bytes
            </p>

            {isOpen && draft ? (
              <div className="space-y-2">
                <p className="text-[11px] text-text-tertiary">
                  Generated guidance is untrusted and is shown as plain text.
                </p>
                <Textarea
                  aria-label="Skill draft"
                  className="min-h-40 font-mono text-[11px]"
                  value={draftText}
                  readOnly={readOnly}
                  onChange={(event) =>
                    setDrafts((current) => ({
                      ...current,
                      [proposal.id]: event.target.value,
                    }))
                  }
                />
                <pre className="max-h-32 overflow-auto whitespace-pre-wrap rounded-md bg-muted/30 p-2 text-[11px]">
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
              <Button
                size="sm"
                disabled={readOnly || review.approve.isPending}
                onClick={async () => {
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
                }}
              >
                {card
                  ? "Confirm publish & attach"
                  : "Publish & request confirmation"}
              </Button>
              <Button
                size="sm"
                disabled={readOnly || review.reject.isPending}
                onClick={() => review.reject.mutate(proposal.id)}
              >
                Reject
              </Button>
            </div>
            {card ? (
              <p className="text-[11px] text-warning">
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
