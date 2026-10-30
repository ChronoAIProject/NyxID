import { useId, useState } from "react";
import { Check, MessageSquare, ThumbsDown, ThumbsUp } from "lucide-react";
import { toast } from "sonner";
import type {
  AssistantFeedbackVote,
  ReplyFeedback,
} from "@/hooks/use-assistant-feedback";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { cn } from "@/lib/utils";

/** The backend's comment bound, in characters. */
const COMMENT_LIMIT = 1000;

const FEEDBACK_REASONS: Readonly<Record<AssistantFeedbackVote, readonly string[]>> = {
  up: ["Useful", "Accurate", "Easy to understand"],
  down: ["Not useful", "Not accurate", "Bad response", "Hard to understand"],
};

function feedbackDetails(comment: string, vote: AssistantFeedbackVote) {
  const separator = comment.indexOf(" — ");
  const prefix = separator < 0 ? comment : comment.slice(0, separator);
  const reasons = prefix.split(", ");
  const known = Object.values(FEEDBACK_REASONS).flat();
  if (!reasons.every((reason) => known.includes(reason))) {
    return { chosen: [], note: comment, comment };
  }
  const chosen = reasons.filter((reason) => FEEDBACK_REASONS[vote].includes(reason));
  const note = separator < 0 ? "" : comment.slice(separator + 3);
  return {
    chosen,
    note,
    comment: chosen.length === reasons.length ? comment : feedbackComment(chosen, note),
  };
}

/**
 * The stored comment: the chosen reasons, then the reader's own words
 * (`Not accurate, Bad response — wrong date`).
 */
function feedbackComment(reasons: readonly string[], note: string): string {
  const tags = reasons.join(", ");
  const text = note.trim();
  return tags && text ? `${tags} — ${text}` : tags || text;
}

/** Helpful and Not helpful, for a reply's action line. */
export function ReplyFeedbackVotes({ feedback }: { readonly feedback: ReplyFeedback }) {
  return (
    <div role="group" aria-label="Reply feedback" className="flex max-w-full flex-wrap items-center gap-1">
      {(["up", "down"] as const).map((next) => {
        const Icon = next === "up" ? ThumbsUp : ThumbsDown;
        return (
          <button
            key={next}
            type="button"
            aria-label={next === "up" ? "Helpful" : "Not helpful"}
            aria-pressed={feedback.vote === next}
            disabled={feedback.pending}
            onClick={() => void feedback.toggle(next)}
            className={cn(
              "rounded p-1 text-text-tertiary hover:bg-overlay disabled:opacity-50",
              feedback.vote === next && (next === "up" ? "text-success" : "text-destructive"),
            )}
          >
            <Icon className="h-3.5 w-3.5" />
          </button>
        );
      })}
      {feedback.vote ? (
        <button type="button" aria-label={feedback.comment ? "Edit feedback comment" : "Add feedback comment"} title={feedback.comment ? "Edit feedback comment" : "Add feedback comment"} disabled={feedback.pending} onClick={feedback.edit} className="rounded p-1 text-text-tertiary hover:bg-overlay disabled:opacity-50">
          <MessageSquare className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
}

/**
 * What follows a vote: the optional details dialog, or the failure to save
 * the vote itself.
 */
export function ReplyFeedbackFollowUp({ feedback }: { readonly feedback: ReplyFeedback }) {
  const vote = feedback.vote;
  return (
    <>
      <Dialog
        open={feedback.formOpen && Boolean(vote)}
        onOpenChange={(open) => {
          if (!open) feedback.close();
        }}
      >
        {vote && feedback.formOpen ? (
          <FeedbackDetails
            key={vote}
            vote={vote}
            initialComment={feedback.comment}
            pending={feedback.pending}
            onSend={feedback.send}
            onClose={feedback.close}
          />
        ) : null}
      </Dialog>
      {feedback.error ? (
        <p role="alert" className="mt-1 text-11 text-destructive">
          Unable to save feedback. Try again.
        </p>
      ) : null}
    </>
  );
}

function FeedbackDetails({
  vote,
  initialComment,
  pending,
  onSend,
  onClose,
}: {
  readonly vote: AssistantFeedbackVote;
  readonly initialComment: string;
  readonly pending: boolean;
  readonly onSend: (comment: string) => Promise<boolean>;
  readonly onClose: () => void;
}) {
  const fieldId = useId();
  const [initial] = useState(() => feedbackDetails(initialComment, vote));
  const [chosen, setChosen] = useState<readonly string[]>(initial.chosen);
  const [note, setNote] = useState(initial.note);
  const [submitting, setSubmitting] = useState(false);
  const [failed, setFailed] = useState(false);
  const options = FEEDBACK_REASONS[vote];
  const unchanged = note === initial.note && chosen.length === initial.chosen.length && chosen.every((reason) => initial.chosen.includes(reason));
  const comment = unchanged ? initial.comment : feedbackComment(options.filter((reason) => chosen.includes(reason)), note);
  const submit = async () => {
    if (pending || submitting || comment.length > COMMENT_LIMIT) return;
    setSubmitting(true);
    const saved = await onSend(comment);
    setSubmitting(false);
    setFailed(!saved);
    if (saved) toast.success("Thanks for the feedback.");
  };
  return (
    <DialogContent className="md:max-w-md">
      <DialogHeader>
        <DialogTitle>
          {vote === "up" ? "Give positive feedback" : "Give negative feedback"}
        </DialogTitle>
        <DialogDescription>
          {vote === "up"
            ? "What did you like about this reply? Choose any that apply."
            : "What went wrong with this reply? Choose any that apply."}
        </DialogDescription>
      </DialogHeader>
      <div role="group" aria-label="Reasons" className="flex flex-wrap gap-2">
        {options.map((reason) => {
          const on = chosen.includes(reason);
          return (
            <button
              key={reason}
              type="button"
              aria-pressed={on}
              onClick={() =>
                setChosen((current) =>
                  on ? current.filter((item) => item !== reason) : [...current, reason],
                )
              }
              className={cn(
                "inline-flex h-8 items-center gap-1.5 rounded-full px-3.5 text-12 font-medium transition-all duration-150 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring active:scale-[0.97]",
                on
                  ? "bg-nyx-secondary-400/15 pl-3 text-foreground ring-1 ring-inset ring-nyx-secondary-400/60"
                  : "text-muted-foreground ring-1 ring-inset ring-hairline-strong hover:bg-overlay-strong hover:text-foreground",
              )}
            >
              {on ? <Check className="size-3.5 text-nyx-secondary-400" aria-hidden /> : null}
              {reason}
            </button>
          );
        })}
      </div>
      <div className="space-y-2">
        <Label htmlFor={fieldId}>Details (optional)</Label>
        <Textarea
          id={fieldId}
          value={note}
          maxLength={COMMENT_LIMIT}
          onChange={(event) => setNote(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter" && (event.metaKey || event.ctrlKey) && !pending && !submitting) {
              event.preventDefault();
              void submit();
            }
          }}
          className="min-h-24 resize-none text-12"
          placeholder={
            vote === "up"
              ? "What was satisfying about this reply?"
              : "What should the reply have done instead?"
          }
        />
      </div>
      {failed ? (
        <p role="alert" className="text-12 text-destructive">
          Unable to save feedback. Try again.
        </p>
      ) : null}
      <DialogFooter>
        <Button type="button" variant="outline" onClick={onClose}>
          Cancel
        </Button>
        <Button
          type="button"
          variant="primary"
          disabled={pending || comment.length > COMMENT_LIMIT}
          isLoading={submitting}
          onClick={() => void submit()}
        >
          Submit
        </Button>
      </DialogFooter>
    </DialogContent>
  );
}
