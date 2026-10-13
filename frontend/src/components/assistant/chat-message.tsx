import {
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
  MessageSquare,
  Sparkles,
  Wrench,
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
import { MachineToolCard } from "./machine-tool-card";
import { useAssistantFeedback } from "@/hooks/use-assistant-feedback";
import { capture } from "@/lib/telemetry";

const EMPTY_MESSAGES: readonly ChatMessage[] = [];

function PulseDot({ className }: { readonly className?: string }) {
  return (
    <span
      aria-hidden
      className={cn(
        "inline-block h-1.5 w-1.5 animate-pulse rounded-full bg-nyx-secondary-400",
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

function ActivityBlock({ message }: { readonly message: ChatMessage }) {
  const steps = message.steps ?? [];
  const tools = (message.toolCalls ?? []).filter((tool) => !tool.machine);
  const count = steps.length + tools.length;
  const [open, setOpen] = useState(false);
  if (!count) return null;
  const running =
    steps.some((step) => step.status === "running") ||
    tools.some((tool) => tool.status === "running");
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
        {running ? (
          <PulseDot />
        ) : tools.some((tool) => tool.status === "error") ||
          steps.some((step) => step.status === "error") ? (
          <X className="h-3 w-3 text-destructive" />
        ) : (
          <Check className="h-3 w-3 text-success" />
        )}
        <span>
          {count} {count === 1 ? "action" : "actions"}
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
              {step.stepType ? (
                <span className="shrink-0 font-mono text-9 text-text-tertiary">
                  {step.stepType}
                </span>
              ) : null}
            </div>
          ))}
          {tools.map((tool) => (
            <details key={tool.id} className="text-11 text-muted-foreground">
              <summary className="flex cursor-pointer list-none items-center gap-2 py-0.5">
                {tool.status === "running" ? (
                  <PulseDot />
                ) : (
                  <Wrench className="h-3 w-3 text-text-tertiary" />
                )}
                <span className="font-mono">{tool.name || tool.id}</span>
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

/** Metadata-only diagnostics of a failed turn, with a copyable ID. */
function ErrorDetails({ detail }: { readonly detail: ErrorDetail }) {
  const diagnosticId = [
    `turn ${detail.turnId}`,
    detail.responseId ? `response ${detail.responseId}` : null,
    detail.upstreamRequestId ? `request ${detail.upstreamRequestId}` : null,
  ]
    .filter(Boolean)
    .join(" / ");
  return (
    <details className="mt-2 text-11 text-text-tertiary">
      <summary className="cursor-pointer">Details</summary>
      <dl className="mt-1 grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 rounded border border-hairline p-2 font-mono">
        <dt>stage</dt>
        <dd>{detail.stage}</dd>
        {detail.httpStatus != null ? (
          <>
            <dt>http</dt>
            <dd>{detail.httpStatus}</dd>
          </>
        ) : null}
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

export function ChatMessageBubble({
  message,
  interactiveCards = true,
  onRetry,
  conversationId,
  feedbackEnabled = true,
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
  readonly onRetry?: () => Promise<void> | void;
  readonly conversationId?: string;
  readonly feedbackEnabled?: boolean;
}) {
  const streaming = message.status === "streaming";
  const [retrying, setRetrying] = useState(false);
  const [retryError, setRetryError] = useState<string>();
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
  if (message.role === "user") {
    return (
      <div className="ml-auto max-w-[78%] rounded-lg bg-overlay-strong px-3 py-2 text-12 leading-relaxed text-foreground whitespace-pre-wrap">
        {message.steering && (
          <p className="mb-1 text-11 text-muted-foreground" role="note">
            Steering ·{" "}
            {steeringNotice(message.steering.code, message.steering.outcome)}
          </p>
        )}
        {content}
        {message.images?.length ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {message.images.map((image) => (
              <ToolImage key={image.id} image={image} />
            ))}
          </div>
        ) : null}
        {feedbackError ? (
          <p role="alert" className="mt-1 text-11 text-destructive">
            Unable to save feedback. Try again.
          </p>
        ) : null}
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
  const runningTool = streaming
    ? message.toolCalls?.filter((tool) => tool.status === "running").at(-1)
    : undefined;
  return (
    <article
      role={thinking ? "status" : undefined}
      aria-label={thinking ? "Assistant is thinking" : undefined}
      className="flex items-start gap-2.5"
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
        {content ? <TextBlock text={content} streaming={streaming} /> : null}
        {message.status === "error" && message.errorDetail ? (
          <ErrorDetails detail={message.errorDetail} />
        ) : null}
        {message.status === "error" && onRetry ? (
          <div className="mt-2">
            <button
              type="button"
              className="text-11 underline disabled:opacity-50"
              disabled={retrying}
              onClick={() => void retry()}
            >
              {retrying ? "Retrying…" : "Retry"}
            </button>
            {retryError ? (
              <p role="alert" className="mt-1 text-11 text-destructive">
                {retryError}
              </p>
            ) : null}
          </div>
        ) : null}
        {message.images?.length ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {message.images
              .filter(
                (image) =>
                  !message.toolCalls?.some(
                    (tool) => tool.machine?.screenshot_id === image.id,
                  ),
              )
              .map((image) => (
                <ToolImage key={image.id} image={image} />
              ))}
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
        {streaming && !content ? (
          <div
            data-streaming-dots
            role="status"
            aria-label="Assistant is answering"
            className="flex h-5 items-center gap-1"
          >
            <PulseDot />
            <PulseDot className="[animation-delay:120ms]" />
            <PulseDot className="[animation-delay:240ms]" />
            {runningTool ? (
              <span
                data-running-tool
                className="ml-1.5 truncate font-mono text-11 text-muted-foreground"
              >
                {runningTool.name}
              </span>
            ) : null}
          </div>
        ) : null}
        {message.status === "error" && message.error ? (
          <div className="mt-2 flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/[0.05] px-3 py-2 text-11 text-destructive">
            <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{message.error}</span>
          </div>
        ) : null}
        {feedbackEnabled &&
        conversationId &&
        !streaming &&
        message.status === "complete" &&
        !message.errorCode &&
        message.id ? (
          <div
            role="group"
            className="mt-2 flex items-center gap-1"
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
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
  readonly conversationId?: string;
  readonly feedbackEnabled?: boolean;
  readonly onRetry?: () => Promise<void> | void;
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

export function ChatMessageList({
  session,
  bottomInset,
  emptyDescription,
  footer,
  notice,
  projectionVersion,
  renderMessage,
  feedbackEnabled = false,
}: {
  readonly session: ChatSessionState | null;
  readonly bottomInset: number;
  readonly emptyDescription?: ReactNode;
  readonly footer?: ReactNode;
  readonly notice?: ReactNode;
  readonly projectionVersion?: string | number;
  readonly renderMessage?: (message: ChatMessage) => ReactNode;
  readonly feedbackEnabled?: boolean;
}) {
  const [detectedMessageId, setDetectedMessageId] = useState<string>();
  const scrollRef = useRef<HTMLDivElement>(null);
  const followingRef = useRef(true);
  const lastScrollTopRef = useRef(0);
  const previousConversationRef = useRef<string | undefined>(undefined);
  const previousUserMessageRef = useRef<string | undefined>(undefined);
  const messages = session?.messages ?? EMPTY_MESSAGES;
  useLayoutEffect(() => {
    const element = scrollRef.current;
    if (!element || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (!followingRef.current) return;
      element.scrollTop = element.scrollHeight;
      lastScrollTopRef.current = element.scrollTop;
    });
    observer.observe(element);
    return () => observer.disconnect();
  }, []);
  const terminalAssistant = messages.at(-1);
  const emptyTerminal = Boolean(
    session &&
    session.status !== "streaming" &&
    session.status !== "draft" &&
    session.status !== "stopped" &&
    terminalAssistant?.role === "assistant" &&
    !terminalAssistant.content.trim() &&
    !terminalAssistant.error &&
    !(
      terminalAssistant.steps?.length ||
      terminalAssistant.toolCalls?.length ||
      terminalAssistant.images?.length
    ) &&
    !(
      terminalAssistant.artifacts?.length ||
      terminalAssistant.authorizationBlockers?.length
    ),
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
    if (conversationChanged || newUserMessage) followingRef.current = true;
    previousConversationRef.current = session?.conversationId;
    previousUserMessageRef.current = latestUserMessageId;
    const element = scrollRef.current;
    if (!element || !followingRef.current) return;
    element.scrollTop = element.scrollHeight;
    lastScrollTopRef.current = element.scrollTop;
  }, [
    bottomInset,
    latestUserMessageId,
    messages,
    projectionVersion,
    session?.conversationId,
  ]);

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

  return (
    <div
      ref={scrollRef}
      onScroll={handleScroll}
      className="min-h-0 flex-1 overflow-y-auto overscroll-contain px-4 sm:px-6"
    >
      <div
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
        {messages.map((message) => (
          <div id={`message-${message.id}`} key={message.id}>
            {renderMessage?.(message) ?? (
              <ChatMessageEntry
                message={message}
                conversationId={session?.conversationId}
                feedbackEnabled={feedbackEnabled}
              />
            )}
          </div>
        ))}
        {footer}
        {emptyTurnDetected ? (
          <span data-empty-turn-error className="sr-only" aria-hidden="true" />
        ) : null}
      </div>
    </div>
  );
}
