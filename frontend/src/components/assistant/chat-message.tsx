import { useRouter, useRouterState } from "@tanstack/react-router";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { isAssistantShellRoute } from "@/lib/assistant/account-panel-search";
import { CAPACITY_ACKNOWLEDGEMENT, formatCapacityWait } from "@/lib/assistant/capacity-wait";
import { useVirtualizer } from "@tanstack/react-virtual";
import {
  memo,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
  type ReactNode,
  type UIEvent,
} from "react";
import {
  Brain,
  Check,
  ChevronRight,
  CircleAlert,
  Copy,
  MessageSquare,
  RotateCcw,
  Sparkles,
  X,
  ThumbsDown,
  ThumbsUp,
} from "lucide-react";
import { ArtifactBlock } from "@/components/assistant/blocks/artifact-block";
import { ConnectCard } from "@/components/assistant/blocks/connect-card";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { ToolImage } from "@/components/assistant/blocks/tool-image";
import { authorizationBlockerToConnectCard } from "@/lib/assistant/chat-authorization";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import type { ChatMessage, ChatSessionState } from "@/lib/assistant/chat-types";
import { cn } from "@/lib/utils";
import { steeringNotice } from "@/lib/assistant/nyxagent-steering";
import type { RuntimeToolCallInfo } from "@/lib/assistant/runtime-event-semantics";
import { describeToolActivity } from "@/lib/assistant/tool-activity-text";
import { MachineToolCard } from "./machine-tool-card";
import { MessageCopyAction } from "./message-copy-action";
import { MessageTime } from "./message-time";
import { useAssistantFeedback } from "@/hooks/use-assistant-feedback";
import { capture } from "@/lib/telemetry";

const EMPTY_MESSAGES: readonly ChatMessage[] = [];

function recoveryReason(reason: string): string {
  switch (reason) {
    case "turn_active": return "Another turn is active. Check its result before retrying.";
    case "already_retried": return "This reply was already retried. Review the newer result.";
    case "agent_destroyed": return "This agent's threads are read-only.";
    case "attachment_expired": return "An original attachment is no longer available.";
    case "activity_outcome_uncertain": return "A tool may have taken effect. Check its result before another attempt.";
    case "activity_evidence_incomplete":
    case "lineage_incomplete": return "Completed actions cannot be verified for this retry.";
    case "input_unavailable": return "The original request is no longer available.";
    case "status_unavailable": return "Retry availability could not be checked. Check again before retrying.";
    default: return "This reply cannot be retried automatically.";
  }
}

function PulseDot({ className }: { readonly className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "inline-block h-1.5 w-1.5 motion-safe:animate-pulse rounded-full bg-nyx-secondary-400",
        className,
      )}
    />
  );
}

function ThinkingBlock({
  text,
  streaming,
}: {
  readonly text: string;
  readonly streaming: boolean;
}) {
  const [open, setOpen] = useState(false);
  if (!text) return null;
  return (
    <div className="mb-2" role="status" aria-label="Assistant reasoning">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex items-center gap-1.5 py-1 text-11 text-text-tertiary transition-colors hover:text-muted-foreground"
      >
        <ChevronRight
          className={cn("h-3 w-3 transition-transform", open && "rotate-90")}
        />
        <Brain className="h-3 w-3" />
        Thinking
        {streaming ? <PulseDot /> : null}
      </button>
      {open ? (
        <div className="ml-3.5 max-h-56 overflow-auto border-l border-hairline pl-3 text-11 leading-relaxed text-text-tertiary whitespace-pre-wrap">
          {text}
        </div>
      ) : null}
    </div>
  );
}

function toolFailureCopy(
  failure: { kind: string; status?: number } | null | undefined,
): string {
  switch (failure?.kind) {
    case "service_status":
      return failure.status == null
        ? "The service returned an error. No HTTP status is available."
        : `The service responded with HTTP ${failure.status}.`;
    case "refused":
      return "NyxID refused this call.";
    case "internal":
      return "NyxID could not complete this call.";
    case "tool_error":
      return "The tool reported an error.";
    default:
      return "This step did not complete. No additional error details are available.";
  }
}

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds}s`;
  return `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`;
}

function ToolElapsed({ startedAt }: { readonly startedAt: string }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.floor((now - Date.parse(startedAt)) / 1000));
  if (seconds < 3) return null;
  return <span aria-hidden> · {formatElapsed(seconds)}</span>;
}

function StatusLabel({ label, interrupted }: {
  readonly label: string;
  readonly interrupted?: boolean;
}) {
  return (
    <span role="status" aria-live="polite">
      {label}
      {interrupted ? (
        <span> Connection interrupted; checking every few seconds</span>
      ) : null}
    </span>
  );
}

function ActivitySummaryIcon({ status }: { readonly status: ChatMessage["status"] }) {
  if (status === "streaming") return <PulseDot />;
  if (status === "error") {
    return <X className="h-3 w-3 text-destructive" aria-label="Turn failed" />;
  }
  if (status === "stopped") {
    return <CircleAlert className="h-3 w-3 text-muted-foreground" aria-label="Stopped" />;
  }
  return <Check className="h-3 w-3 text-success" aria-label="Completed" />;
}

function activitySummary(message: ChatMessage): string {
  switch (message.status) {
    case "streaming":
      return message.turnStatus?.label ?? "Working…";
    case "error":
      return "Turn failed";
    case "stopped":
      return "Stopped";
    default:
      return "Completed";
  }
}

function ToolStatusIcon({
  status,
}: {
  readonly status: RuntimeToolCallInfo["status"];
}) {
  if (status === "running") return <PulseDot className="mx-[3px]" />;
  if (status === "error") {
    return (
      <X aria-label="Failed" className="h-3 w-3 shrink-0 text-destructive" />
    );
  }
  return (
    <Check aria-label="Succeeded" className="h-3 w-3 shrink-0 text-success" />
  );
}

function ActivityBlock({ message }: { readonly message: ChatMessage }) {
  const steps = message.steps ?? [];
  const tools = (message.toolCalls ?? []).filter((tool) => !tool.machine);
  const count = steps.length + tools.length;
  const total = Math.max(count, message.activityTotal ?? 0);
  const [open, setOpen] = useState(false);
  if (!count) return null;
  const failed = steps.filter((step) => step.status === "error").length
    + tools.filter((tool) => tool.status === "error").length;
  const summary = activitySummary(message);
  return (
    <div className="mb-2 overflow-hidden rounded-lg border border-hairline bg-overlay/35">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        aria-expanded={open}
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-11 text-muted-foreground hover:bg-overlay"
      >
        <ChevronRight
          className={cn("h-3 w-3 transition-transform", open && "rotate-90")}
        />
        <ActivitySummaryIcon status={message.status} />
        <span>
          {summary} · {total} {total === 1 ? "action" : "actions"}
          {message.activityTotal && message.activityTotal > count ? (
            <span className="ml-1">· latest {count} of {message.activityTotal} shown</span>
          ) : null}
          {failed ? <span className="ml-1 text-warning">· {failed} failed</span> : null}
        </span>
      </button>
      {open ? (
        <div className="space-y-1 border-t border-hairline px-2.5 py-2">
          {steps.map((step) => (
            <div key={step.id} className="flex items-start gap-2 text-11">
              {step.status === "running" ? (
                <PulseDot className="mt-1" />
              ) : step.status === "error" ? (
                <X className="mt-0.5 h-3 w-3 text-destructive" />
              ) : (
                <Check className="mt-0.5 h-3 w-3 text-success" />
              )}
              <span className="min-w-0 flex-1 break-words text-muted-foreground">
                {step.name || "Processing"}
              </span>
              {step.status === "error" ? (
                <span className="text-text-tertiary">
                  Step failed. The assistant can continue after a failed step.
                </span>
              ) : null}
              {step.stepType ? (
                <span className="shrink-0 font-mono text-9 text-text-tertiary">
                  {step.stepType}
                </span>
              ) : null}
            </div>
          ))}
          {tools.map((tool) => (
            <details key={tool.id} className="text-11 text-muted-foreground">
              <summary
                data-tool-status={tool.status}
                className="flex cursor-pointer list-none items-center gap-2 py-0.5"
              >
                <ToolStatusIcon status={tool.status} />
                <span
                  className={cn(
                    "min-w-0 flex-1 truncate",
                    tool.status === "error" && "text-destructive",
                  )}
                >
                  {describeToolActivity(tool.name || tool.id, tool.status)}
                </span>
                <span className="max-w-[45%] shrink-0 truncate font-mono text-10 text-text-tertiary">
                  {tool.name || tool.id}
                </span>
                {tool.status === "error" ? (
                  <span className="text-text-tertiary">
                    {toolFailureCopy(tool.failure)} The assistant can continue
                    after a failed step.
                  </span>
                ) : null}
              </summary>
              {tool.result ? (
                <pre className="ml-5 mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded-md bg-overlay px-2 py-1.5 font-mono text-10 text-text-tertiary">
                  {tool.result.slice(0, 500)}
                </pre>
              ) : null}
            </details>
          ))}
        </div>
      ) : null}
    </div>
  );
}

type ErrorDetail = NonNullable<ChatMessage["errorDetail"]>;

function BillingRemedy() {
  const router = useRouter({ warn: false });
  return router ? <BillingRemedyWithRouter /> : null;
}

function BillingRemedyWithRouter() {
  const pathname = useRouterState({ select: (state) => state.location.pathname });
  const panel = useAccountPanel();
  if (!isAssistantShellRoute(pathname)) return null;
  return <button type="button" className="mt-2 block text-11 underline"
    onClick={() => void panel.open("billing", { panelTab: "overview" })}>Open Billing &amp; Usage</button>;
}

function CapacityDetails({ attempts, capacityWaitMs }: { readonly attempts?: number | null; readonly capacityWaitMs?: number | null }) {
  return <>{attempts != null ? <><dt>attempts</dt><dd>{attempts}</dd></> : null}
    {capacityWaitMs != null && capacityWaitMs > 0 ? <><dt>capacity</dt><dd>waited {formatCapacityWait(capacityWaitMs)} for capacity</dd></> : null}</>;
}

/** Metadata-only diagnostics of a failed turn, with a copyable ID. */
function ErrorDetails({ detail }: { readonly detail: ErrorDetail }) {
  const diagnosticId = [
    `turn ${detail.turnId}`,
    detail.attempts != null ? `attempts ${detail.attempts}` : null,
    detail.capacityWaitMs != null ? `capacity_wait_ms ${detail.capacityWaitMs}` : null,
    detail.upstreamCode ? `upstream ${detail.upstreamCode}` : null,
    detail.responseId ? `response ${detail.responseId}` : null,
    detail.upstreamRequestId ? `request ${detail.upstreamRequestId}` : null,
  ]
    .filter(Boolean)
    .join(" / ");
  return (
    <details className="mt-2 text-11 text-text-tertiary">
      <summary className="cursor-pointer">Details</summary>
      <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 rounded border border-hairline p-2 font-mono">
        <CapacityDetails attempts={detail.attempts} capacityWaitMs={detail.capacityWaitMs} />
        <dt>stage</dt>
        <dd>{detail.stage}</dd>
        {detail.httpStatus != null ? (
          <>
            <dt>http</dt>
            <dd>{detail.httpStatus}</dd>
          </>
        ) : null}
        {detail.upstreamCode ? <><dt>upstream</dt><dd>{detail.upstreamCode}</dd></> : null}
        <dt>turn</dt>
        <dd>{detail.turnId}</dd>
        {detail.responseId ? (
          <>
            <dt>response</dt>
            <dd>{detail.responseId}</dd>
          </>
        ) : null}
        {detail.upstreamRequestId ? (
          <>
            <dt>request</dt>
            <dd>{detail.upstreamRequestId}</dd>
          </>
        ) : null}
        <dt>continuations</dt>
        <dd>{detail.continuations}</dd>
        {detail.occurredAt ? (
          <>
            <dt>at</dt>
            <dd>
              <time dateTime={detail.occurredAt}>{new Date(detail.occurredAt).toLocaleString()}</time>
            </dd>
          </>
        ) : null}
      </dl>
      <button
        type="button"
        className="mt-1 underline"
        onClick={() => void navigator.clipboard?.writeText(diagnosticId)}
      >
        Copy diagnostic ID
      </button>
    </details>
  );
}

/**
 * What the reply is waiting on, with the seconds since it started waiting: the
 * model can think for many seconds before its first word, which must not read
 * as a frozen page. Ticks only itself, never the transcript.
 */
function WaitingStatus({ label, interrupted, capacity, capacityStartedAt, capacityDeadline }: {
  readonly label?: string;
  readonly interrupted?: boolean;
  readonly capacity?: boolean;
  readonly capacityStartedAt?: string | null;
  readonly capacityDeadline?: string | null;
}) {
  const [started] = useState(Date.now);
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.floor((now - started) / 1000));
  if (capacity) {
    const from = Date.parse(capacityStartedAt ?? "");
    const to = Date.parse(capacityDeadline ?? "");
    const clock = (value: number) => `${Math.floor(value / 60)}:${String(value % 60).padStart(2, "0")}`;
    const bound = Math.max(0, Math.ceil((to - from) / 1000));
    const waited = Math.min(bound, Math.max(0, Math.floor((now - from) / 1000)));
    return (
      <span data-waiting-status>
        <StatusLabel label="Waiting for the assistant" interrupted={interrupted} />
        {Number.isFinite(from) && Number.isFinite(to)
          ? <span aria-hidden> · {clock(waited)} of up to {clock(bound)}</span>
          : seconds >= 3 ? <span aria-hidden> · {formatElapsed(seconds)}</span> : null}
      </span>
    );
  }
  return (
    <span data-waiting-status={!label && seconds >= 3 ? true : undefined}>
      <StatusLabel label={label ?? "Thinking"} interrupted={interrupted} />
      {seconds >= 3 ? <span aria-hidden> · {formatElapsed(seconds)}</span> : null}
    </span>
  );
}

export function ChatMessageBubble({
  message,
  interactiveCards = true,
  onRetry,
  onCheckRecovery,
  conversationId,
  feedbackEnabled = true,
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
  readonly onRetry?: () => Promise<void> | void;
  readonly onCheckRecovery?: () => Promise<unknown> | void;
  readonly conversationId?: string;
  readonly feedbackEnabled?: boolean;
}) {
  const streaming = message.status === "streaming";
  const capacityNote = streaming && message.systemNote === "capacity_wait";
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState<string>();
  const [reviewOpen, setReviewOpen] = useState(false);
  const [copyState, setCopyState] = useState<"copied" | "failed">();
  const [checking, setChecking] = useState(false);
  const [checkError, setCheckError] = useState<string>();
  const retry = async () => {
    if (!onRetry) return;
    setRetrying(true);
    setRetryError(undefined);
    try {
      await onRetry();
    } catch (error) {
      setRetryError(error instanceof Error && error.message ? error.message : "Retry failed. Try again.");
    } finally {
      setRetrying(false);
    }
  };
  const content =
    message.role === "assistant"
      ? sanitizeAssistantMessageContent(message.content)
      : message.content;
  const [vote, setVote] = useState<"up" | "down" | undefined>(
    message.feedback?.vote,
  );
  const [commentOpen, setCommentOpen] = useState(false);
  const [comment, setComment] = useState(message.feedback?.comment ?? "");
  const [feedbackError, setFeedbackError] = useState(false);
  const feedback = useAssistantFeedback(
    "conversation",
    conversationId ?? "",
    message.id,
  );
  const setFeedback = async (next: "up" | "down") => {
    const previous = vote;
    const selected = previous === next ? undefined : next;
    setVote(selected);
    if (!conversationId || !feedbackEnabled) return;
    try {
      await feedback.mutateAsync(selected ? { vote: selected } : {});
      setFeedbackError(false);
    } catch {
      setVote(previous);
      setFeedbackError(true);
      return;
    }
    if (selected)
      capture({
        name: "ui.assistant_feedback_submitted",
        props: { message_id: message.id, vote: selected },
      });
  };
  if (message.continuation_of) {
    return <p role="status" className="px-[7px] text-11 text-muted-foreground">Continuing after your decision</p>;
  }
  if (message.role === "user") {
    return (
      <div className="group/message ml-auto flex max-w-[78%] flex-col items-end gap-1">
        <div className="rounded-lg bg-overlay-strong px-3 py-2 text-12 leading-relaxed text-foreground whitespace-pre-wrap">
          {message.steering && (
            <p className="mb-1 text-11 text-muted-foreground" role="note">
              Steering ·{" "}
              {steeringNotice(message.steering.code, message.steering.outcome)}
            </p>
          )}
          {message.images?.length ? (
            <div className={cn("flex flex-wrap gap-2", content && "mb-2")}>
              {message.images.map((image) => (
                <ToolImage key={image.id} image={image} />
              ))}
            </div>
          ) : null}
          {content}
          {feedbackError ? (
            <p role="alert" className="mt-1 text-11 text-destructive">
              Unable to save feedback. Try again.
            </p>
          ) : null}
        </div>
        <div className="flex items-center gap-1"><MessageCopyAction text={content} /><MessageTime value={message.timestamp} reveal /></div>
      </div>
    );
  }
  const printable = Boolean(
    content ||
    message.steps?.length ||
    message.toolCalls?.length ||
    message.images?.length ||
    message.artifacts?.length ||
    message.authorizationBlockers?.length,
  );
  const thinking = streaming && !printable;
  // Machine screenshots render inside their tool card.
  const replyImages = (message.images ?? []).filter(
    (image) => !message.toolCalls?.some((tool) => tool.machine?.screenshot_id === image.id),
  );
  const runningTool = streaming
    ? message.toolCalls?.filter((tool) => tool.status === "running").at(-1)
    : undefined;
  const lastTool = streaming ? message.toolCalls?.at(-1) : undefined;
  let statusLabel: string | undefined;
  if (message.turnStatus && message.turnStatus.phase !== "tool") {
    statusLabel = message.turnStatus.label;
  } else if (runningTool) {
    statusLabel = `${describeToolActivity(runningTool.name, "running")}…`;
  } else if (lastTool) {
    statusLabel = describeToolActivity(lastTool.name, lastTool.status);
  } else {
    statusLabel = message.turnStatus?.label;
  }
  const toolStartedAt = message.turnStatus?.phase === "tool"
    ? message.turnStatus.toolStartedAt
    : undefined;
  const showFeedback = Boolean(
    feedbackEnabled &&
    conversationId &&
    message.status === "complete" &&
    !message.errorCode &&
    !message.systemNote &&
    message.id,
  );
  return (
    <article
      role={thinking ? "status" : undefined}
      aria-label={capacityNote ? "Assistant is waiting for capacity" : thinking ? "Assistant is thinking" : undefined}
      data-system-note={capacityNote ? "capacity_wait" : undefined}
      className="group/message flex items-start gap-2.5"
    >
      <span
        {...(thinking ? { "data-assistant-halo": "" } : {})}
        aria-hidden="true"
        className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-nyx-secondary-400/25 bg-nyx-secondary-400/10 text-nyx-secondary-400"
      >
        <Sparkles className="h-3 w-3" />
      </span>
      <div className="min-w-0 max-w-[min(84%,758px)] flex-1 pt-0.5">
        <ThinkingBlock text={message.thinking ?? ""} streaming={streaming} />
        <ActivityBlock message={message} />
        {message.toolCalls
          ?.filter((tool) => tool.machine)
          .map((tool) => (
            <MachineToolCard
              key={tool.id}
              receipt={tool.machine!}
              conversationId={tool.conversationId}
              images={message.images}
            />
          ))}
        {replyImages.length ? (
          <div className={cn("flex flex-wrap gap-2", content ? "mb-2" : "mt-2")}>
            {replyImages.map((image) => (
              <ToolImage key={image.id} image={image} />
            ))}
          </div>
        ) : null}
        {capacityNote ? (
          <p role="note" className="text-12 leading-relaxed text-foreground">{CAPACITY_ACKNOWLEDGEMENT}</p>
        ) : content ? <TextBlock text={content} streaming={streaming} /> : null}
        {message.status === "error" && ["wallet_suspended", "plan_entitlement_required"].includes(message.errorCode ?? "") ? (
          <BillingRemedy />
        ) : null}
        {message.status === "complete" && message.turnMetadata && ((message.turnMetadata.capacityWaitMs ?? 0) > 0 || (message.turnMetadata.attempts ?? 0) > 1) ? (
          <details className="mt-2 text-11 text-text-tertiary"><summary className="cursor-pointer">Details</summary>
            <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 rounded border border-hairline p-2 font-mono">
              <CapacityDetails {...message.turnMetadata} />
            </dl>
          </details>
        ) : null}
        {message.role === "assistant" && message.channelDelivery ? <p className="mt-2 text-11 text-text-tertiary" role="note">{message.channelDelivery.status === "interrupted_prefix" && message.channelDelivery.heard_prefix_matches_sent ? `Caller interrupted after a verified prefix of ${message.channelDelivery.heard_prefix_chars ?? 0} characters. The remaining answer is unconfirmed.` : message.channelDelivery.status === "delivered" ? "Provider confirmed message delivery." : message.channelDelivery.status === "failed" ? "Provider reported delivery failure." : "Generated reply; caller hearing or message delivery is unconfirmed."}</p> : null}
        {message.status === "error" && message.errorDetail ? (
          <ErrorDetails detail={message.errorDetail} />
        ) : null}
        {message.status === "error" && onRetry ? (
          <div className="mt-2">
            <button
              type="button"
              className="inline-flex items-center gap-1 text-11 underline disabled:opacity-50"
              disabled={retrying}
              onClick={() => void retry()}
            >
              <RotateCcw className="size-3" aria-hidden />
              {retrying ? "Retrying…" : "Retry"}
            </button>
            {retryError ? (
              <p role="alert" className="mt-1 text-11 text-destructive">
                {retryError}
              </p>
            ) : null}
          </div>
        ) : null}
        {message.status === "error" && message.recovery?.status === "review" ? (
          <div className="mt-2 space-y-2 text-11 text-muted-foreground">
            <p>{recoveryReason(message.recovery.reason)}</p>
            {message.recovery.text !== null ? (
              <>
                <button type="button" className="inline-flex items-center gap-1 underline"
                  onClick={() => setReviewOpen((open) => !open)} aria-expanded={reviewOpen}>
                  <ChevronRight className={cn("size-3", reviewOpen && "rotate-90")} aria-hidden />
                  Review request
                </button>
                {reviewOpen ? (
                  <div className="max-h-56 overflow-auto rounded-md border border-border p-2">
                    <p className="whitespace-pre-wrap break-words text-foreground">{message.recovery.text}</p>
                    {message.recovery.attachments.length ? (
                      <p className="mt-2 break-all">Attachments: {message.recovery.attachments.join(", ")}</p>
                    ) : null}
                    <button type="button" className="mt-2 inline-flex items-center gap-1 underline"
                      onClick={() => {
                        const text = message.recovery?.text;
                        if (text === null || text === undefined) return;
                        void (async () => {
                          try {
                            await navigator.clipboard.writeText(text);
                            setCopyState("copied");
                          } catch {
                            setCopyState("failed");
                          }
                        })();
                      }}>
                      <Copy className="size-3" aria-hidden /> Copy request
                    </button>
                    {copyState ? <p role="status" className="mt-1">
                      {copyState === "copied" ? "Request copied." : "Copy failed. Select the text above."}
                    </p> : null}
                  </div>
                ) : null}
              </>
            ) : null}
          </div>
        ) : null}
        {message.status === "error" && onCheckRecovery ? (
          <div className="mt-2 text-11">
            <button type="button" className="inline-flex items-center gap-1 underline disabled:opacity-50"
              disabled={checking} onClick={() => {
                setChecking(true);
                setCheckError(undefined);
                void Promise.resolve().then(onCheckRecovery).catch((error: unknown) => {
                  setCheckError(error instanceof Error ? error.message : "Could not check retry availability.");
                }).finally(() => setChecking(false));
              }}>
              <RotateCcw className="size-3" aria-hidden />
              {checking ? "Checking…" : "Check retry availability"}
            </button>
            {checkError ? <p role="alert" className="mt-1 text-destructive">{checkError}</p> : null}
          </div>
        ) : null}
        {interactiveCards
          ? message.authorizationBlockers?.map((blocker) => (
              <div className="mt-2" key={blocker.serviceSlug}>
                <ConnectCard
                  block={authorizationBlockerToConnectCard(blocker)}
                />
              </div>
            ))
          : null}
        {message.artifacts?.map((artifact) => (
          <div className="mt-2" key={artifact.block_id}>
            <ArtifactBlock block={artifact} />
          </div>
        ))}
        {streaming ? (
          <div
            data-turn-status
            className="flex min-h-5 items-center gap-1 text-11 text-muted-foreground"
          >
            {!content ? <span data-streaming-dots><PulseDot /></span> : null}
            <span
              {...(!content && runningTool ? { "data-running-tool": runningTool.name } : {})}
              {...(!runningTool && lastTool ? { "data-last-tool": true } : {})}
            >
              {toolStartedAt && !(capacityNote || message.waitingForCapacity) ? (
                <>
                  <StatusLabel
                    label={statusLabel ?? "Thinking"}
                    interrupted={message.turnStatus?.interrupted}
                  />
                  <ToolElapsed startedAt={toolStartedAt} />
                </>
              ) : (
                <WaitingStatus
                  label={statusLabel}
                  interrupted={message.turnStatus?.interrupted}
                  capacity={capacityNote || message.waitingForCapacity}
                  capacityStartedAt={message.capacityWaitStartedAt}
                  capacityDeadline={message.capacityWaitDeadline}
                />
              )}
            </span>
          </div>
        ) : null}
        {message.status === "stopped" ? (
          <p
            role="status"
            aria-label="Turn stopped"
            className="flex min-h-5 items-center gap-1 text-11 text-muted-foreground"
          >
            <CircleAlert aria-hidden="true" className="h-3 w-3" />
            Stopped
          </p>
        ) : null}
        {message.status === "error" && message.error ? (
          <div className="mt-2 flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/[0.05] px-3 py-2 text-11 text-destructive">
            <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{message.error}</span>
          </div>
        ) : null}
        {/* The time joins the feedback line once the reply settles; a
            streaming reply has no settled instant yet. */}
        {streaming ? null : (
          <div className="mt-2 flex flex-wrap items-center gap-1 empty:hidden">
            <MessageCopyAction text={content} />
            {showFeedback ? (
              <div
                role="group"
                className="flex items-center gap-1"
                aria-label="Reply feedback"
              >
                <button
                  type="button"
                  aria-label="Helpful"
                  aria-pressed={vote === "up"}
                  disabled={feedback.isPending}
                  onClick={() => void setFeedback("up")}
                  className={cn(
                    "rounded p-1 text-text-tertiary hover:bg-overlay",
                    vote === "up" && "text-success",
                  )}
                >
                  <ThumbsUp className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  aria-label="Not helpful"
                  aria-pressed={vote === "down"}
                  disabled={feedback.isPending}
                  onClick={() => void setFeedback("down")}
                  className={cn(
                    "rounded p-1 text-text-tertiary hover:bg-overlay",
                    vote === "down" && "text-destructive",
                  )}
                >
                  <ThumbsDown className="h-3.5 w-3.5" />
                </button>
                <button
                  type="button"
                  aria-label="Add feedback comment"
                  onClick={() => setCommentOpen((open) => !open)}
                  className="ml-1 rounded px-2 py-1 text-10 text-text-tertiary hover:bg-overlay"
                >
                  Comment
                </button>
              </div>
            ) : null}
            <MessageTime
              value={message.timestamp}
              reveal
              className={showFeedback ? "ml-1" : undefined}
            />
          </div>
        )}
        {feedbackEnabled && conversationId && commentOpen && !streaming ? (
          <div className="mt-2 flex max-w-sm gap-2 rounded-lg border border-border bg-overlay p-2">
            <textarea
              autoFocus
              aria-label="Optional feedback comment"
              onKeyDown={(event) => {
                if (event.key === "Escape") setCommentOpen(false);
              }}
              value={comment}
              maxLength={1000}
              onChange={(event) => setComment(event.target.value)}
              className="min-h-14 flex-1 resize-y bg-transparent text-11 outline-none"
              placeholder="Optional comment"
            />
            <button
              type="button"
              disabled={!vote || feedback.isPending}
              className="self-end rounded bg-nyx-secondary-400/20 px-2 py-1 text-10 disabled:opacity-50"
              onClick={() => {
                if (!conversationId || !vote) return;
                void feedback
                  .mutateAsync({ vote, comment })
                  .then(() => {
                    setFeedbackError(false);
                    setCommentOpen(false);
                  })
                  .catch(() => setFeedbackError(true));
              }}
            >
              Send
            </button>
          </div>
        ) : null}
        {feedbackError ? (
          <p role="alert" className="mt-1 text-11 text-destructive">
            Unable to save feedback. Try again.
          </p>
        ) : null}
      </div>
    </article>
  );
}

export function ChatMessageEntry({
  message,
  interactiveCards = true,
  conversationId,
  feedbackEnabled = true,
  onRetry,
  onCheckRecovery,
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
  readonly conversationId?: string;
  readonly feedbackEnabled?: boolean;
  readonly onRetry?: () => Promise<void> | void;
  readonly onCheckRecovery?: () => Promise<unknown> | void;
}) {
  if (
    message.role === "system" &&
    message.id.startsWith("nyxagent-context-reset:")
  ) {
    return (
      <p
        role="note"
        aria-label="Conversation context reset"
        className="text-xs text-text-tertiary"
      >
        {message.content}
      </p>
    );
  }
  const authorName = message.authorName?.trim() ?? "";
  if (message.role === "user" || message.role === "assistant") {
    return (
      <div className="flex flex-col gap-1">
        {authorName ? (
          <div
            className={cn(
              "text-10 text-text-tertiary",
              message.role === "user" ? "self-end" : "ml-8",
            )}
          >
            {authorName}
          </div>
        ) : null}
        <ChatMessageBubble
          message={message}
          interactiveCards={interactiveCards}
          conversationId={conversationId}
          feedbackEnabled={feedbackEnabled}
          onRetry={onRetry}
          onCheckRecovery={onCheckRecovery}
        />
      </div>
    );
  }
  const roleLabel = message.role.trim() || "Message";
  const displayName = authorName || roleLabel;
  return (
    <article
      aria-label={`${displayName} ${roleLabel} message`}
      className="flex gap-2.5"
    >
      <div className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-hairline bg-overlay text-text-tertiary">
        <MessageSquare className="h-3 w-3" />
      </div>
      <div className="min-w-0 max-w-[84%] flex-1">
        <div className="text-11 font-medium text-foreground">{displayName}</div>
        {message.thinking ? (
          <p className="mt-1 text-11 italic text-text-tertiary">
            {message.thinking}
          </p>
        ) : null}
        {message.content ? (
          <div className="mt-1 text-12 leading-relaxed text-foreground whitespace-pre-wrap">
            {message.content}
          </div>
        ) : null}
      </div>
    </article>
  );
}

function EmptyState({ children }: { readonly children?: ReactNode }) {
  return (
    <div className="flex flex-1 items-center justify-center px-6 text-center text-12 text-text-tertiary">
      {children ??
        "Ask NyxID to help with services, access, and account operations."}
    </div>
  );
}

interface MessageRowProps {
  readonly message: ChatMessage;
  readonly conversationId: string | undefined;
  readonly feedbackEnabled: boolean;
  readonly renderMessage?: (message: ChatMessage) => ReactNode;
  readonly projectionVersion?: string | number;
  /** Windowed lists measure each mounted row. */
  readonly index?: number;
  readonly measure?: (element: HTMLDivElement | null) => void;
}

/**
 * A row renders again only when its message or the projection changes: a
 * streamed token re-renders the streaming row, not the whole transcript.
 * `renderMessage` is rebuilt on every parent render, so it must derive its
 * output from the message and state named in `projectionVersion`.
 */
const MessageRow = memo(
  function MessageRow({
    message,
    conversationId,
    feedbackEnabled,
    renderMessage,
    index,
    measure,
  }: MessageRowProps) {
    return (
      <div id={`message-${message.id}`} data-index={index} ref={measure}>
        {renderMessage?.(message) ?? (
          <ChatMessageEntry
            message={message}
            conversationId={conversationId}
            feedbackEnabled={feedbackEnabled}
          />
        )}
      </div>
    );
  },
  (previous, next) =>
    previous.message === next.message &&
    previous.conversationId === next.conversationId &&
    previous.feedbackEnabled === next.feedbackEnabled &&
    previous.projectionVersion === next.projectionVersion &&
    previous.index === next.index &&
    previous.measure === next.measure &&
    Boolean(previous.renderMessage) === Boolean(next.renderMessage),
);

/**
 * Longer transcripts mount only the rows near the viewport, so a long thread
 * opens, streams and scrolls with a bounded DOM. Shorter ones stay whole, which
 * keeps the browser's find-in-page working where it matters most.
 */
export const WINDOW_AFTER_MESSAGES = 30;
/** `gap-4` at default display spacing; the rendered gap is read back. */
const DEFAULT_ROW_GAP_PX = 16;
const ROW_OVERSCAN = 6;

/** A first guess at a row's height; real heights replace it once measured. */
function estimateRowHeight(message: ChatMessage | undefined): number {
  if (!message) return 80;
  const lines = Math.ceil(message.content.length / (message.role === "user" ? 70 : 95));
  return Math.min(2400, (message.role === "user" ? 44 : 64) + 20 * lines);
}

/** `#message-<id>` links (voice transcripts) may point at an unmounted row. */
function hashMessageId(): string | undefined {
  const hash = typeof window === "undefined" ? "" : window.location.hash;
  if (!hash.startsWith("#message-")) return undefined;
  try {
    return decodeURIComponent(hash.slice("#message-".length));
  } catch {
    return undefined;
  }
}

export function ChatMessageList({
  session,
  bottomInset,
  emptyDescription,
  footer,
  notice,
  projectionVersion,
  renderMessage,
  feedbackEnabled = false,
  focusMessageId,
}: {
  readonly session: ChatSessionState | null;
  readonly bottomInset: number;
  readonly emptyDescription?: ReactNode;
  readonly footer?: ReactNode;
  readonly notice?: ReactNode;
  readonly projectionVersion?: string | number;
  readonly renderMessage?: (message: ChatMessage) => ReactNode;
  readonly feedbackEnabled?: boolean;
  /** A row to bring into view once (a linked review card), even when unmounted. */
  readonly focusMessageId?: string;
}) {
  const [detectedMessageId, setDetectedMessageId] = useState<string>();
  const scrollRef = useRef<HTMLDivElement>(null);
  const contentRef = useRef<HTMLDivElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const followingRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const previousConversationRef = useRef<string | undefined>(undefined);
  const previousUserMessageRef = useRef<string | undefined>(undefined);
  const previousWindowedRef = useRef(false);
  const messages = session?.messages ?? EMPTY_MESSAGES;
  // Windowing starts when a thread opens long, or while the reader follows the
  // newest message; never under a reader who scrolled up, whose place a mode
  // switch would lose. Once on, it stays on for that thread.
  const long = messages.length > WINDOW_AFTER_MESSAGES;
  const conversationKey = session?.conversationId ?? session?.clientId;
  const modeRef = useRef<{ readonly conversation?: string; readonly windowed: boolean }>({
    windowed: false,
  });
  if (modeRef.current.conversation !== conversationKey) {
    modeRef.current = { conversation: conversationKey, windowed: long };
  } else if (!modeRef.current.windowed && long && followingRef.current) {
    modeRef.current = { conversation: conversationKey, windowed: true };
  }
  const windowed = modeRef.current.windowed;
  const [scrollMargin, setScrollMargin] = useState(0);
  const [rowGap, setRowGap] = useState(DEFAULT_ROW_GAP_PX);
  const virtualizer = useVirtualizer({
    count: messages.length,
    enabled: windowed,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => estimateRowHeight(messages[index]),
    getItemKey: (index) => messages[index]?.id ?? index,
    overscan: ROW_OVERSCAN,
    gap: rowGap,
    scrollMargin,
    // Keep the rows on screen in place when older messages load above them.
    anchorTo: "end",
    // A transcript opens at its newest message.
    initialOffset: () => (followingRef.current ? Number.MAX_SAFE_INTEGER : 0),
    useFlushSync: false,
  });
  const windowedRef = useRef(windowed);
  windowedRef.current = windowed;

  /**
   * A jump (opening a thread, sending) tells a windowed list where it is at
   * once, so the newest rows render without a blank frame. Following a growing
   * reply only moves the scrollbar: the rows at the end are already mounted.
   */
  const pinToBottom = (jump: boolean) => {
    const element = scrollRef.current;
    if (!element) return;
    if (jump && windowedRef.current) virtualizer.scrollToEnd();
    else element.scrollTop = element.scrollHeight;
    lastScrollTopRef.current = element.scrollTop;
  };
  const pinRef = useRef(pinToBottom);
  pinRef.current = pinToBottom;

  useLayoutEffect(() => {
    const element = scrollRef.current;
    const content = contentRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    // The viewport resizing, a streamed reply growing, or an image loading.
    const observer = new ResizeObserver(() => {
      // Display spacing scales the gap between rows; keep the windowed
      // geometry on the rendered value.
      if (content) {
        const gap = Number.parseFloat(getComputedStyle(content).rowGap);
        if (Number.isFinite(gap)) setRowGap((current) => (current === gap ? current : gap));
      }
      if (followingRef.current) pinRef.current(false);
    });
    observer.observe(element);
    if (content) observer.observe(content);
    return () => observer.disconnect();
  }, []);
  // Rows are positioned below the notice; the virtualizer needs that offset.
  useLayoutEffect(() => {
    const offset = windowed ? (listRef.current?.offsetTop ?? 0) : 0;
    if (offset !== scrollMargin) setScrollMargin(offset);
  }, [notice, scrollMargin, windowed]);
  const terminalAssistant = messages.at(-1);
  const emptyTerminal = Boolean(
    session &&
      session.status !== "streaming" &&
      session.status !== "draft" &&
      session.status !== "stopped" &&
      terminalAssistant?.role === "assistant" &&
      terminalAssistant.status !== "stopped" &&
      !terminalAssistant.content.trim() &&
      !terminalAssistant.error &&
      !(
        terminalAssistant.steps?.length ||
        terminalAssistant.toolCalls?.length ||
        terminalAssistant.images?.length
      ) &&
      !(terminalAssistant.artifacts?.length ||
        terminalAssistant.authorizationBlockers?.length),
  );
  useEffect(() => {
    if (!emptyTerminal) return;
    const messageId = terminalAssistant?.id;
    const timer = setTimeout(() => setDetectedMessageId(messageId), 700);
    return () => clearTimeout(timer);
  }, [emptyTerminal, terminalAssistant?.id]);
  const emptyTurnDetected =
    emptyTerminal && detectedMessageId === terminalAssistant?.id;

  const latestUserMessageId = [...messages]
    .reverse()
    .find((message) => message.role === "user" && !message.steering)?.id;
  useLayoutEffect(() => {
    const conversationChanged =
      previousConversationRef.current !== session?.conversationId;
    const newUserMessage = Boolean(
      latestUserMessageId &&
      previousUserMessageRef.current &&
      previousUserMessageRef.current !== latestUserMessageId,
    );
    const windowedChanged = previousWindowedRef.current !== windowed;
    const jump = conversationChanged || newUserMessage || windowedChanged;
    if (conversationChanged || newUserMessage) followingRef.current = true;
    previousConversationRef.current = session?.conversationId;
    previousUserMessageRef.current = latestUserMessageId;
    previousWindowedRef.current = windowed;
    if (followingRef.current) pinRef.current(jump);
  }, [
    windowed,
    bottomInset,
    latestUserMessageId,
    messages,
    projectionVersion,
    session?.conversationId,
  ]);

  // Bring a linked row into view once, mounting it first when it is windowed out.
  const focusIndex = focusMessageId
    ? messages.findIndex((message) => message.id === focusMessageId)
    : -1;
  const focusedRef = useRef<string | undefined>(undefined);
  useEffect(() => {
    if (!focusMessageId) {
      focusedRef.current = undefined;
      return;
    }
    if (focusIndex < 0 || focusedRef.current === focusMessageId) return;
    focusedRef.current = focusMessageId;
    followingRef.current = false;
    if (windowed) virtualizer.scrollToIndex(focusIndex, { align: "center" });
  }, [focusIndex, focusMessageId, virtualizer, windowed]);
  const messagesRef = useRef(messages);
  messagesRef.current = messages;
  const revealedHashRef = useRef<string | undefined>(undefined);
  const revealHash = useCallback(() => {
    const id = hashMessageId();
    const index = id ? messagesRef.current.findIndex((message) => message.id === id) : -1;
    if (index < 0 || !windowedRef.current) return;
    revealedHashRef.current = id;
    followingRef.current = false;
    virtualizer.scrollToIndex(index, { align: "center" });
  }, [virtualizer]);
  useEffect(() => {
    window.addEventListener("hashchange", revealHash);
    return () => window.removeEventListener("hashchange", revealHash);
  }, [revealHash]);
  // A link opened with the hash already set: reveal it once its row has loaded.
  const hashTargetLoaded = messages.some((message) => message.id === hashMessageId());
  useEffect(() => {
    if (hashTargetLoaded && windowed && revealedHashRef.current !== hashMessageId()) revealHash();
  }, [hashTargetLoaded, revealHash, windowed]);

  function handleScroll(event: UIEvent<HTMLDivElement>) {
    const element = event.currentTarget;
    const distance =
      element.scrollHeight - element.clientHeight - element.scrollTop;
    if (distance <= 48) followingRef.current = true;
    else if (element.scrollTop < lastScrollTopRef.current - 1) {
      followingRef.current = false;
    }
    lastScrollTopRef.current = element.scrollTop;
  }

  const rows = windowed
    ? virtualizer.getVirtualItems().map((item) => ({ index: item.index, start: item.start }))
    : messages.map((_, index) => ({ index, start: 0 }));
  const firstStart = rows[0]?.start ?? scrollMargin;

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="relative min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 sm:px-6"
      // The windowed list anchors rows itself; the browser's anchoring would double it.
      style={windowed ? { overflowAnchor: "none" } : undefined}
    >
      <div
        ref={contentRef}
        className="mx-auto flex min-h-full w-full max-w-[758px] flex-col gap-4 pt-6"
        style={{ paddingBottom: Math.max(bottomInset + 24, 96) }}
      >
        {notice ? (
          <div
            role="status"
            className="rounded-lg border border-border bg-overlay px-3 py-2 text-11 text-muted-foreground"
          >
            {notice}
          </div>
        ) : null}
        {!messages.length ? <EmptyState>{emptyDescription}</EmptyState> : null}
        {/* One element tree in both modes, so crossing the threshold never
            remounts a row (or restarts a streaming reply's reveal). */}
        <div
          ref={listRef}
          className={windowed ? "relative w-full" : "contents"}
          style={windowed ? { height: virtualizer.getTotalSize() } : undefined}
        >
          <div
            className={windowed ? "absolute inset-x-0 top-0 flex flex-col gap-4" : "contents"}
            style={
              windowed ? { transform: `translateY(${String(firstStart - scrollMargin)}px)` } : undefined
            }
          >
            {rows.map(({ index }) => {
              const message = messages[index]!;
              return (
                <MessageRow
                  key={message.id}
                  message={message}
                  conversationId={session?.conversationId}
                  feedbackEnabled={feedbackEnabled}
                  renderMessage={renderMessage}
                  projectionVersion={projectionVersion}
                  index={windowed ? index : undefined}
                  measure={windowed ? virtualizer.measureElement : undefined}
                />
              );
            })}
          </div>
        </div>
        {footer}
        {emptyTurnDetected ? (
          <span data-empty-turn-error className="sr-only" aria-hidden="true" />
        ) : null}
      </div>
    </div>
  );
}
