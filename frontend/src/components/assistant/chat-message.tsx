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
  X,
} from "lucide-react";
import { ArtifactBlock } from "@/components/assistant/blocks/artifact-block";
import { ConnectCard } from "@/components/assistant/blocks/connect-card";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { ToolImage } from "@/components/assistant/blocks/tool-image";
import { authorizationBlockerToConnectCard } from "@/lib/assistant/chat-authorization";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import type {
  ChatMessage,
  ChatSessionState,
} from "@/lib/assistant/chat-types";
import { cn } from "@/lib/utils";
import { steeringNotice } from "@/lib/assistant/nyxagent-steering";
import { MachineToolCard } from "./machine-tool-card";

const EMPTY_MESSAGES: readonly ChatMessage[] = [];

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

function toolFailureCopy(failure: { kind: string; status?: number } | null | undefined): string {
  switch (failure?.kind) {
    case "service_status": return `The service responded with HTTP ${failure.status}.`;
    case "refused": return "NyxID refused this call.";
    case "internal": return "NyxID could not complete this call.";
    case "tool_error": return "The tool reported an error.";
    default: return "This step did not complete. No additional error details are available.";
  }
}

function ToolElapsed({ startedAt }: { readonly startedAt: string }) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(timer);
  }, []);
  const seconds = Math.max(0, Math.floor((now - Date.parse(startedAt)) / 1000));
  return <span>{seconds < 60 ? `${seconds}s` : `${Math.floor(seconds / 60)}:${String(seconds % 60).padStart(2, "0")}`}</span>;
}

function ActivityBlock({ message }: { readonly message: ChatMessage }) {
  const steps = message.steps ?? [];
  const tools = (message.toolCalls ?? []).filter((tool) => !tool.machine);
  const count = steps.length + tools.length;
  const total = Math.max(count, message.activityTotal ?? 0);
  const [open, setOpen] = useState(false);
  if (!count) return null;
  const failed = steps.filter((step) => step.status === "error").length + tools.filter((tool) => tool.status === "error").length;
  const active = message.status === "streaming";
  const summary = active
    ? message.turnStatus?.label ?? "Working…"
    : message.status === "error"
      ? "Turn failed"
      : message.status === "stopped"
        ? "Stopped"
        : "Completed";
  const summaryIcon = active ? <PulseDot /> : message.status === "error" ? <X className="h-3 w-3 text-destructive" aria-label="Turn failed" /> : message.status === "stopped" ? <CircleAlert className="h-3 w-3 text-muted-foreground" aria-label="Stopped" /> : <Check className="h-3 w-3 text-success" aria-label="Completed" />;
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
        {summaryIcon}
        <span>{summary} · {total} {total === 1 ? "action" : "actions"}{message.activityTotal && message.activityTotal > count ? <span className="ml-1">· latest {count} of {message.activityTotal} shown</span> : null}{!active && failed ? <span className="ml-1 text-warning">· {failed} failed</span> : null}</span>
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
              {step.status === "error" ? <span className="text-text-tertiary">Step failed. The assistant can continue after a failed step.</span> : null}
              {step.stepType ? (
                <span className="shrink-0 font-mono text-9 text-text-tertiary">
                  {step.stepType}
                </span>
              ) : null}
            </div>
          ))}
          {tools.map((tool) => (
            <details key={tool.id} className="text-11 text-muted-foreground">
              <summary className="flex cursor-pointer list-none items-center gap-2 py-0.5" {...(tool.status === "running" && message.status === "streaming" && !message.content ? { "data-running-tool": tool.name || tool.id } : {})}>
                {tool.status === "running" ? (
                  <PulseDot />
                ) : (
                  tool.status === "error" ? <X className="h-3 w-3 text-warning" aria-label="Failed tool" /> : <Check className="h-3 w-3 text-success" aria-label="Succeeded tool" />
                )}
                <span className="font-mono" {...(tool.status === "running" ? { "data-running-tool": tool.name || tool.id } : {})}>{tool.name || tool.id}</span>
                {tool.status === "error" ? <span className="text-text-tertiary">{toolFailureCopy(tool.failure)} The assistant can continue after a failed step.</span> : null}
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

export function ChatMessageBubble({
  message,
  interactiveCards = true,
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
}) {
  const streaming = message.status === "streaming";
  const content =
    message.role === "assistant"
      ? sanitizeAssistantMessageContent(message.content)
      : message.content;
  if (message.role === "user") {
    return (
      <div className="ml-auto max-w-[78%] rounded-lg bg-overlay-strong px-3 py-2 text-12 leading-relaxed text-foreground whitespace-pre-wrap">
        {message.steering && <p className="mb-1 text-11 text-muted-foreground" role="note">
          Steering · {steeringNotice(message.steering.code, message.steering.outcome)}
        </p>}
        {content}
        {message.images?.length ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {message.images.map((image) => <ToolImage key={image.id} image={image} />)}
          </div>
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
  const runningTool = message.toolCalls?.find((tool) => tool.status === "running");
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
        {message.toolCalls?.filter((tool) => tool.machine).map((tool) => (
          <MachineToolCard key={tool.id} receipt={tool.machine!} conversationId={tool.conversationId} images={message.images} />
        ))}
        {content ? <TextBlock text={content} streaming={streaming} /> : null}
        {message.images?.length ? (
          <div className="mt-2 flex flex-wrap gap-2">
            {message.images.filter((image) => !message.toolCalls?.some((tool) => tool.machine?.screenshot_id === image.id)).map((image) => (
              <ToolImage key={image.id} image={image} />
            ))}
          </div>
        ) : null}
        {interactiveCards
          ? message.authorizationBlockers?.map((blocker) => (
              <div className="mt-2" key={blocker.serviceSlug}>
                <ConnectCard block={authorizationBlockerToConnectCard(blocker)} />
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
            <span role="status" aria-live="polite" {...(!content && runningTool ? { "data-running-tool": runningTool.name } : {})}>{message.turnStatus?.label ?? (runningTool ? `Running ${runningTool.name}` : "Working…")}{message.turnStatus?.interrupted ? <span> Connection interrupted; checking every few seconds</span> : null}</span>
            {message.turnStatus?.toolStartedAt ? <ToolElapsed startedAt={message.turnStatus.toolStartedAt} /> : null}
          </div>
        ) : null}
        {message.status === "stopped" ? (
          <p role="status" aria-label="Turn stopped" className="flex min-h-5 items-center gap-1 text-11 text-muted-foreground">
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
      </div>
    </article>
  );
}

export function ChatMessageEntry({
  message,
  interactiveCards = true,
}: {
  readonly message: ChatMessage;
  readonly interactiveCards?: boolean;
}) {
  if (message.role === "system" && message.id.startsWith("nyxagent-context-reset:")) {
    return (
      <p role="note" aria-label="Conversation context reset" className="text-xs text-text-tertiary">
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
        />
      </div>
    );
  }
  const roleLabel = message.role.trim() || "Message";
  const displayName = authorName || roleLabel;
  return (
    <article aria-label={`${displayName} ${roleLabel} message`} className="flex gap-2.5">
      <div className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-hairline bg-overlay text-text-tertiary">
        <MessageSquare className="h-3 w-3" />
      </div>
      <div className="min-w-0 max-w-[84%] flex-1">
        <div className="text-11 font-medium text-foreground">{displayName}</div>
        {message.thinking ? (
          <p className="mt-1 text-11 italic text-text-tertiary">{message.thinking}</p>
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
      {children ?? "Ask NyxID to help with services, access, and account operations."}
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
}: {
  readonly session: ChatSessionState | null;
  readonly bottomInset: number;
  readonly emptyDescription?: ReactNode;
  readonly footer?: ReactNode;
  readonly notice?: ReactNode;
  readonly projectionVersion?: string | number;
  readonly renderMessage?: (message: ChatMessage) => ReactNode;
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
    const distance = element.scrollHeight - element.clientHeight - element.scrollTop;
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
            {renderMessage?.(message) ?? <ChatMessageEntry message={message} />}
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
