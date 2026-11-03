import {
  GroupedList,
  GroupedRow,
  GroupedInput,
} from "@/components/ui/grouped-list";
import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { Switch } from "@/components/ui/switch";
import { useAgentLearningReview } from "@/hooks/use-agent-learning-review";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { publicationFailureText } from "@/lib/assistant/skill-publication-copy";
import { learningDraftEditable } from "@/schemas/agent-skills";
import { AuthoredSkillScopePreview } from "./authored-skill-scope-preview";
import { LearningDraftEditor } from "./learning-draft-editor";

function requestFailure(cause: unknown): string {
  return cause instanceof Error && cause.message
    ? cause.message
    : "The request could not be completed. Check the proposal and try again.";
}

export function AgentLearningReview({
  agentId,
  proposalId,
  readOnly = false,
}: {
  readonly agentId: string;
  readonly proposalId?: string;
  readonly readOnly?: boolean;
}) {
  const enabled = useFeature(FEATURE_FLAG.AGENT_LEARNING);
  const review = useAgentLearningReview(agentId, enabled);
  const [open, setOpen] = useState<string | undefined>(proposalId);
  useEffect(() => {
    if (proposalId && review.data) document.getElementById(`learning-proposal-${proposalId}`)?.scrollIntoView?.({ block: "nearest" });
  }, [proposalId, review.data]);
  const [cards, setCards] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [threshold, setThreshold] = useState(15);

  const serverThreshold = review.status.data?.threshold;
  useEffect(() => {
    if (serverThreshold !== undefined) setThreshold(serverThreshold);
  }, [serverThreshold]);

  if (!enabled || (readOnly && !review.data)) {
    return null;
  }

  const learningEnabled = review.status.data?.enabled ?? false;
  const proposals = review.data?.proposals ?? [];

  return (
    <section aria-label="Learning" className="space-y-3">
      <GroupedList
        title="Learning proposals"
        footer={
          <>
            <p>
              Completed work can produce private, untrusted Ornn skill drafts.
              Full access publishes and attaches automatically; other modes
              require your confirmation.
            </p>
            {review.error ? (
              <p role="alert" className="text-destructive">
                Automatic learning is unavailable.
              </p>
            ) : null}
            {review.status.data?.publication_blocked ? (
              <p role="alert" className="text-warning">
                Publishing is unavailable for this organization because it has
                no owner binding. Organization members may consent to learning,
                but only the bound owner can publish private Ornn skills.
                Contact an organization owner to publish.
              </p>
            ) : null}
            {[
              review.run.error,
              review.configure.error,
              review.consent?.error,
              review.reject.error,
            ]
              .filter(Boolean)
              .map((error, index) => (
                <p key={index} role="alert" className="text-destructive">
                  {requestFailure(error)}
                </p>
              ))}
          </>
        }
      >
        <GroupedRow
          title="Automatic learning"
          accessory={
            readOnly ? (
              learningEnabled ? (
                "On"
              ) : (
                "Off"
              )
            ) : (
              <Switch
                aria-label="Enable automatic learning"
                checked={learningEnabled}
                disabled={review.configure.isPending}
                onCheckedChange={(checked) =>
                  review.configure.mutate({
                    enabled: checked,
                    threshold: Math.min(50, Math.max(1, threshold)),
                  })
                }
              />
            )
          }
        />
        {!readOnly ? (
          <GroupedRow
            title="Threshold"
            accessory={
              <GroupedInput
                aria-label="Learning threshold"
                className="w-16 text-right"
                type="number"
                min={1}
                max={50}
                value={threshold}
                onChange={(event) =>
                  setThreshold(Number(event.target.value) || 15)
                }
                onBlur={() => {
                  if (learningEnabled)
                    review.configure.mutate({
                      enabled: true,
                      threshold: Math.min(50, Math.max(1, threshold)),
                    });
                }}
              />
            }
          />
        ) : null}
        {review.status.data?.org_owned && !readOnly ? (
          <GroupedRow
            title="I consent to learning proposals for this organization"
            accessory={
              <Switch
                aria-label="I consent to learning proposals for this organization"
                checked={review.status.data.opted_in ?? false}
                disabled={review.consent?.isPending}
                onCheckedChange={(checked) => review.consent?.mutate(checked)}
              />
            }
          />
        ) : null}
        {!proposals.length ? (
          <GroupedRow title="No pending proposals." />
        ) : null}
        {!readOnly ? (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={review.run.isPending}
              onClick={() => review.run.mutate()}
            >
              Run learning now
            </button>
          </GroupedRow>
        ) : null}
      </GroupedList>

      {proposals.map((proposal) => {
        const draft = proposal.draft;
        const isOpen = open === proposal.id;
        const card = cards[proposal.id];
        const checkOnly = proposal.evidence_available === false;
        // An older NyxID prepared this package; it is rebuilt, never published.
        const legacyPackage =
          proposal.failure_code === "legacy_package_requires_reprepare";
        // A verified version NyxID refused to attach is settled; the server
        // refuses to check it again.
        const settled =
          proposal.status === "published_unpinned" &&
          (proposal.failure_code === "evidence_unavailable" ||
            proposal.failure_code === "base_changed");
        // A publication that reached Ornn but is not attached yet resumes
        // through the same confirmed approve route.
        const recovering =
          !settled &&
          (proposal.status === "published_unpinned" ||
            proposal.status === "publishing");
        const approvable =
          !settled &&
          (checkOnly ||
            recovering ||
            proposal.status === "pending" ||
            proposal.status === "publication_failed");
        const summary = (
          <GroupedList
            title="Proposal"
            footer={
              <>
                {proposal.failure_code ? (
                  <p role="alert" className="text-destructive">
                    {publicationFailureText(
                      proposal.failure_code,
                      proposal.status,
                      proposal.source,
                    )}
                  </p>
                ) : null}
                {errors[proposal.id] ? (
                  <p role="alert" className="text-destructive">
                    {errors[proposal.id]}
                  </p>
                ) : null}
                {checkOnly ? (
                  <p role="status" className="text-warning">
                    Learning evidence or consent is no longer available. NyxID
                    can only check whether this publication reached Ornn; it
                    will not attach it to the agent.
                  </p>
                ) : null}
              </>
            }
          >
            <GroupedRow
              title={draft?.name ?? "Learning proposal"}
              description={draft?.description}
              accessory={proposal.status.replaceAll("_", " ")}
            />
            <GroupedRow
              title="Evidence"
              accessory={`${proposal.evidence_count} items · ${proposal.body_bytes.toLocaleString()} bytes`}
            />
            {proposal.published_version ? (
              <GroupedRow
                title="Published version"
                accessory={proposal.published_version}
              />
            ) : null}
          </GroupedList>
        );
        const actions = (
          <GroupedList
            footer={
              approvable &&
              proposal.source !== "authored" &&
              !legacyPackage &&
              !checkOnly &&
              !readOnly ? (
                <p className="text-warning">
                  {card
                    ? "Step 2 of 2: Confirming this exact card publishes the private skill and attaches its pinned version. The card expires if the draft or agent changes."
                    : "Publishing takes two steps: this button first asks for an owner confirmation card, then you confirm that exact card here. Nothing is published until you confirm."}
                </p>
              ) : undefined
            }
          >
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() => setOpen(isOpen ? undefined : proposal.id)}
              >
                {isOpen ? "Hide draft" : "Review draft"}
              </button>
            </GroupedRow>
            {proposal.source === "authored" && proposal.status !== "pinned" ? (
              proposal.card_conversation_id ? (
                <GroupedRow asChild interactive>
                  <Link
                    to="/assistant"
                    search={{
                      c: proposal.card_conversation_id,
                      card: proposal.acknowledgement_id ?? undefined,
                    }}
                    className="text-11 text-primary hover:underline"
                  >
                    Open the confirmation card
                  </Link>
                </GroupedRow>
              ) : (
                <GroupedRow title="Confirm this authored skill from its conversation card." />
              )
            ) : legacyPackage ? (
              <GroupedRow asChild interactive>
                <button
                  type="button"
                  disabled={readOnly || review.reprepare.isPending}
                  onClick={async () => {
                    setErrors((current) => ({ ...current, [proposal.id]: "" }));
                    try {
                      await review.reprepare.mutateAsync({
                        proposalId: proposal.id,
                      });
                    } catch (cause) {
                      setErrors((current) => ({
                        ...current,
                        [proposal.id]: requestFailure(cause),
                      }));
                    }
                  }}
                >
                  Review updated package
                </button>
              </GroupedRow>
            ) : !approvable ? null : (
              <GroupedRow asChild interactive>
                <button
                  type="button"
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
                    : recovering
                      ? card
                        ? "Confirm finish publishing"
                        : "Finish publishing"
                      : proposal.status === "publication_failed"
                        ? "Retry publication"
                        : card
                          ? "Confirm publish & attach"
                          : "Publish & request confirmation"}
                </button>
              </GroupedRow>
            )}
            {/* The server rejects only pending or failed drafts. A failed
                  publication that may have reached Ornn is refused, and the
                  reason appears in the error line above. */}
            {proposal.status === "pending" ||
            proposal.status === "publication_failed" ||
            (proposal.auto_allowed && proposal.status === "pinned") ? (
              <GroupedRow asChild interactive>
                <button
                  type="button"
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
                  {proposal.auto_allowed && proposal.status === "pinned"
                    ? "Discard and detach"
                    : "Reject"}
                </button>
              </GroupedRow>
            ) : null}
          </GroupedList>
        );
        return (
          <article
            key={proposal.id}
            id={`learning-proposal-${proposal.id}`}
            className="space-y-3"
          >
            {!isOpen ? summary : null}
            <GroupedDetail
              open={isOpen}
              title={draft?.name ?? "Learning proposal"}
              backLabel="Back to Skills"
              onBack={() => setOpen(undefined)}
            >
              {isOpen ? summary : null}
              {isOpen &&
              proposal.source === "authored" &&
              proposal.auto_allowed ? (
                <AuthoredSkillScopePreview
                  agentId={agentId}
                  proposalId={proposal.id}
                />
              ) : null}
              {isOpen && draft ? (
                <LearningDraftEditor
                  key={`${proposal.id}:${proposal.revision}`}
                  agentId={agentId}
                  proposal={{ ...proposal, draft }}
                  readOnly={readOnly || !learningDraftEditable(proposal.status)}
                  edit={review.edit}
                  validate={review.validate}
                />
              ) : null}

              {isOpen ? actions : null}
            </GroupedDetail>
            {!isOpen ? actions : null}
          </article>
        );
      })}
    </section>
  );
}
