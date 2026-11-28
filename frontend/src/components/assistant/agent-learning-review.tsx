import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { getFormattingLocale } from "@/lib/formatting-locale";
import { useDrawerSearchEntries } from "@/lib/drawer-search";
import {
  GroupedList,
  GroupedRow,
  GroupedInput,
} from "@/components/ui/grouped-list";
import { useEffect, useState } from "react";
import { Link } from "@tanstack/react-router";
import { GroupedDetail } from "@/components/ui/grouped-detail";
import { Button } from "@/components/ui/button";
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
  useUiTranslation();
  const enabled = useFeature(FEATURE_FLAG.AGENT_LEARNING);
  const review = useAgentLearningReview(agentId, enabled);
  const [open, setOpen] = useState<string | undefined>(proposalId);
  useEffect(() => {
    if (proposalId && review.data) document.getElementById(`learning-proposal-${proposalId}`)?.scrollIntoView?.({ block: "nearest" });
  }, [proposalId, review.data]);
  const [cards, setCards] = useState<Record<string, string>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  const [threshold, setThreshold] = useState("15");
  const [thresholdError, setThresholdError] = useState(false);
  const [thresholdEdited, setThresholdEdited] = useState(false);

  const serverThreshold = review.status.data?.threshold;
  useEffect(() => {
    if (serverThreshold !== undefined) setThreshold(String(serverThreshold));
  }, [serverThreshold]);

  const configurationReady = Boolean(review.status.data) && !review.status.isPending && !review.status.error;
  function configureLearning(nextEnabled: boolean) {
    if (!configurationReady || review.configure.isPending) return;
    const value = Number(threshold);
    if (!Number.isInteger(value) || value < 1 || value > 50) {
      review.configure.reset?.();
      setThreshold(String(serverThreshold ?? 15));
      setThresholdError(true);
      return;
    }
    setThresholdError(false);
    if (nextEnabled === review.status.data?.enabled && value === serverThreshold) {
      setThreshold(String(value));
      setThresholdEdited(false);
      return;
    }
    review.configure.mutate({ enabled: nextEnabled, threshold: value }, {
      onSuccess: (accepted) => { setThreshold(String(accepted.threshold)); setThresholdEdited(false); },
      onError: () => { setThreshold(String(serverThreshold ?? 15)); setThresholdEdited(false); },
    });
  }

  useDrawerSearchEntries(enabled && (!readOnly || review.data) ? (review.data?.proposals ?? []).map((proposal) => ({
    id: `learning-${proposal.id}`, title: proposal.draft?.name ?? translateUi("Learning proposal"), breadcrumb: translateUi("Skills") + " › " + translateUi("Learning proposals"),
    keywords: `${proposal.status} ${proposal.source}`, pages: [`learning-${proposal.id}`], reveal: () => setOpen(proposal.id),
  })) : []);
  if (!enabled || (readOnly && !review.data)) {
    return null;
  }

  const learningEnabled = review.status.data?.enabled ?? false;
  const proposals = review.data?.proposals ?? [];

  return (
    <section aria-label={translateUi("Learning")} className="space-y-3">
      <GroupedList
        title={translateUi("Learning proposals")}
        search={{ id: "learning-proposals", title: translateUi("Learning proposals"), breadcrumb: "Skills", keywords: "Automatic learning private drafts" }}
        footer={
          <>
            <p>{translateUi("Learning analyzes eligible conversations and may suggest a private skill draft, or find nothing to propose. Enabling learning does not publish learned drafts.")}</p>
            {!readOnly ? <p>{translateUi("Learning settings apply immediately. Discard in the drawer does not undo them.")}</p> : null}
            {review.status.error ? <div className="space-y-2"><p role="alert">{translateUi("Learning settings could not be loaded. Retry before changing them.")}</p><Button type="button" variant="outline" onClick={() => void review.status.refetch()}>{translateUi("Retry")}</Button></div> : null}
            {thresholdError ? <p role="alert">{translateUi("Enter a whole number from 1 to 50. The saved threshold has been restored.")}</p> : null}
            {review.configure.isPending || review.consent?.isPending ? <p role="status">{translateUi("Saving learning settings…")}</p> : configurationReady && !thresholdEdited && !thresholdError && !review.configure.error && !review.consent?.error && (review.configure.isSuccess || review.consent?.isSuccess) ? <p role="status">{translateUi("Learning settings saved.")}</p> : null}
            {review.run.isPending ? <p role="status">{translateUi("Analyzing eligible conversations…")}</p> : review.run.isSuccess ? <p role="status">{translateUi("Analysis requested. It may produce a proposal or find nothing to suggest.")}</p> : null}
            {review.error ? (
              <p role="alert" className="text-destructive">{translateUi("Automatic learning is unavailable.")}</p>
            ) : null}
            {review.status.data?.publication_blocked ? (
              <p role="alert" className="text-warning">{translateUi("Publishing is unavailable for this organization because it has no owner binding. Organization members may consent to learning, but only the bound owner can publish private Ornn skills. Contact an organization owner to publish.")}</p>
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
          search={{ id: "automatic-learning", title: translateUi("Automatic learning"), breadcrumb: translateUi("Skills") + " › " + translateUi("Learning proposals"), keywords: learningEnabled ? "On" : "Off" }}
          title={translateUi("Automatic learning")}
          accessory={
            !configurationReady ? <span role="status">{translateUi(review.status.error ? "Unavailable" : "Loading learning settings…")}</span> : readOnly ? (
              translateUi(learningEnabled ? "On" : "Off")
            ) : (
              <Switch
                aria-label={translateUi("Enable automatic learning")}
                checked={learningEnabled}
                disabled={review.configure.isPending}
                onCheckedChange={configureLearning}
              />
            )
          }
        />
        {!readOnly ? (
          <GroupedRow
            search={{ id: "learning-threshold", title: translateUi("Analyze after eligible conversations"), breadcrumb: translateUi("Skills") + " › " + translateUi("Learning proposals"), keywords: `Threshold ${threshold}` }}
            title={translateUi("Analyze after")}
            description={translateUi("Distinct eligible conversations; each analysis may produce no proposal.")}
            accessory={
              <span className="flex items-center gap-2">
              <GroupedInput
                aria-label={translateUi("Eligible conversations before analysis")}
                className="w-16 text-right"
                type="number"
                min={1}
                max={50}
                step={1}
                disabled={!configurationReady || !learningEnabled || review.configure.isPending}
                value={threshold}
                onChange={(event) => {
                  setThreshold(event.target.value);
                  setThresholdEdited(true);
                  setThresholdError(false);
                }}
                onBlur={() => {
                  if (learningEnabled)
                    configureLearning(true);
                }}
              />
              <span>{translateUi("eligible conversations")}</span>
              </span>
            }
          />
        ) : null}
        {review.status.data?.org_owned && !readOnly ? (
          <GroupedRow
            title={translateUi("I consent to learning proposals for this organization")}
            accessory={
              <Switch
                aria-label={translateUi("I consent to learning proposals for this organization")}
                checked={review.status.data.opted_in ?? false}
                disabled={review.consent?.isPending}
                onCheckedChange={(checked) => review.consent?.mutate(checked)}
              />
            }
          />
        ) : null}
        {!proposals.length ? (
          <GroupedRow title={translateUi("No pending proposals.")} />
        ) : null}
        {!readOnly ? (
          <GroupedRow asChild interactive>
            <button
              type="button"
              disabled={review.run.isPending}
              onClick={() => review.run.mutate()}
            >{translateUi("Run learning now")}</button>
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
            title={translateUi("Proposal")}
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
                  <p role="status" className="text-warning">{translateUi("Learning evidence or consent is no longer available. NyxID can only check whether this publication reached Ornn; it will not attach it to the agent.")}</p>
                ) : null}
              </>
            }
          >
            <GroupedRow
              title={draft?.name ?? translateUi("Learning proposal")}
              description={draft?.description}
              accessory={proposal.status.replaceAll("_", " ")}
            />
            <GroupedRow
              title={translateUi("Evidence")}
              accessory={`${proposal.evidence_count} items · ${proposal.body_bytes.toLocaleString(getFormattingLocale())} bytes`}
            />
            {proposal.published_version ? (
              <GroupedRow
                title={translateUi("Published version")}
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
                    ? translateUi("Confirming this exact card authorizes publication and attachment. The card expires if the draft or agent changes.")
                    : translateUi("This request may publish the private skill and attach it immediately if the approving chat uses Full access. Otherwise, NyxID returns a confirmation card here. The approving chat's mode may differ from this agent's mode.")}
                </p>
              ) : undefined
            }
          >
            <GroupedRow asChild interactive>
              <button
                type="button"
                onClick={() => setOpen(isOpen ? undefined : proposal.id)}
              >
                {isOpen ? translateUi("Hide draft") : translateUi("Review draft")}
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
                    className="type-row-title text-primary hover:underline"
                  >{translateUi("Open the confirmation card")}</Link>
                </GroupedRow>
              ) : (
                <GroupedRow title={translateUi("Confirm this authored skill from its conversation card.")} />
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
                >{translateUi("Review updated package")}</button>
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
                      ? translateUi("Confirm check")
                      : translateUi("Check publication")
                    : recovering
                      ? card
                        ? translateUi("Confirm finish publishing")
                        : translateUi("Finish publishing")
                      : card
                        ? translateUi("Confirm publish & attach")
                        : proposal.status === "publication_failed"
                          ? translateUi("Retry publication")
                          : translateUi("Publish & attach")}
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
                    ? translateUi("Discard and detach")
                    : translateUi("Reject")}
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
              searchPage={`learning-${proposal.id}`}
              open={isOpen}
              title={draft?.name ?? translateUi("Learning proposal")}
              info="Review this proposed skill and its supporting evidence before publishing or dismissing it."
              onReveal={() => setOpen(proposal.id)}
              backLabel={translateUi("Back to Skills")}
              onBack={() => setOpen(undefined)}
            >
              {isOpen ? <div data-drawer-search-target={`learning-${proposal.id}`}>{summary}</div> : null}
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
