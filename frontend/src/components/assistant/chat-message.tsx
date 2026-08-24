import { useEffect, useState, type ReactNode } from "react";
import {
  Brain,
  Check,
  ChevronRight,
  CircleAlert,
  MessageSquare,
  ShieldAlert,
  Sparkles,
  Wrench,
  X,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { sanitizeAssistantMessageContent } from "@/lib/assistant/chat-content";
import type {
  ChatMessage,
  ChatSessionState,
} from "@/lib/assistant/chat-types";
import { cn } from "@/lib/utils";

type InterventionAction =
  | { readonly kind: "resume"; readonly value?: string }
  | { readonly kind: "approve"; readonly value?: string }
  | { readonly kind: "reject"; readonly value?: string }
  | { readonly kind: "signal"; readonly value?: string };

export interface ChatMessageActions {
  readonly onApproval?: (requestId: string, approved: boolean) => void;
  readonly onIntervention?: (action: InterventionAction) => void;
}

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
        className="flex items-center gap-1.5 py-1 text-[11px] text-text-tertiary transition-colors hover:text-muted-foreground"
      >
        <ChevronRight
          className={cn("h-3 w-3 transition-transform", open && "rotate-90")}
        />
        <Brain className="h-3 w-3" />
        Thinking
        {streaming ? <PulseDot /> : null}
      </button>
      {open ? (
        <div className="ml-3.5 max-h-56 overflow-auto border-l border-hairline pl-3 text-[11px] leading-relaxed text-text-tertiary whitespace-pre-wrap">
          {text}
        </div>
      ) : null}
    </div>
  );
}

function ActivityBlock({ message }: { readonly message: ChatMessage }) {
  const steps = message.steps ?? [];
  const tools = message.toolCalls ?? [];
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
        className="flex w-full items-center gap-2 px-2.5 py-1.5 text-left text-[11px] text-muted-foreground hover:bg-overlay"
      >
        <ChevronRight
          className={cn("h-3 w-3 transition-transform", open && "rotate-90")}
        />
        {running ? <PulseDot /> : <Check className="h-3 w-3 text-success" />}
        <span>{count} {count === 1 ? "action" : "actions"}</span>
      </button>
      {open ? (
        <div className="space-y-1 border-t border-hairline px-2.5 py-2">
          {steps.map((step) => (
            <div key={step.id} className="flex items-start gap-2 text-[11px]">
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
                <span className="shrink-0 font-mono text-[9px] text-text-tertiary">
                  {step.stepType}
                </span>
              ) : null}
            </div>
          ))}
          {tools.map((tool) => (
            <details key={tool.id} className="text-[11px] text-muted-foreground">
              <summary className="flex cursor-pointer list-none items-center gap-2 py-0.5">
                {tool.status === "running" ? (
                  <PulseDot />
                ) : (
                  <Wrench className="h-3 w-3 text-text-tertiary" />
                )}
                <span className="font-mono">{tool.name || tool.id}</span>
              </summary>
              {tool.result ? (
                <pre className="ml-5 mt-1 max-h-32 overflow-auto whitespace-pre-wrap rounded-md bg-overlay px-2 py-1.5 font-mono text-[10px] text-text-tertiary">
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

function RuntimeApprovalCard({
  message,
  onApproval,
}: {
  readonly message: ChatMessage;
  readonly onApproval?: ChatMessageActions["onApproval"];
}) {
  const approval = message.pendingApproval;
  if (!approval) return null;
  return (
    <section className="mb-2 overflow-hidden rounded-lg border border-warning/30 bg-warning/[0.05]">
      <div className="flex items-start gap-2 border-b border-warning/20 px-3 py-2.5">
        <ShieldAlert className="mt-0.5 h-4 w-4 text-warning" />
        <div className="min-w-0 flex-1">
          <h3 className="text-[12px] font-semibold text-foreground">
            Tool approval required
          </h3>
          <p className="mt-0.5 break-words font-mono text-[10px] text-muted-foreground">
            {approval.toolName || approval.toolCallId || approval.requestId}
          </p>
        </div>
      </div>
      {approval.argumentsJson ? (
        <pre className="max-h-40 overflow-auto whitespace-pre-wrap px-3 py-2 font-mono text-[10px] text-muted-foreground">
          {approval.argumentsJson}
        </pre>
      ) : null}
      <div className="flex justify-end gap-2 border-t border-warning/20 px-3 py-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          onClick={() => onApproval?.(approval.requestId, false)}
        >
          <X /> Reject
        </Button>
        <Button
          type="button"
          size="sm"
          onClick={() => onApproval?.(approval.requestId, true)}
        >
          <Check /> Approve
        </Button>
      </div>
    </section>
  );
}

function RunInterventionCard({
  message,
  onSubmit,
}: {
  readonly message: ChatMessage;
  readonly onSubmit?: ChatMessageActions["onIntervention"];
}) {
  const intervention = message.pendingRunIntervention;
  const [value, setValue] = useState("");
  if (!intervention) return null;
  const approval = intervention.kind === "human_approval";
  const signal = intervention.kind === "wait_signal";
  return (
    <section className="mb-2 rounded-lg border border-border bg-card p-3">
      <h3 className="text-[12px] font-semibold text-foreground">
        {approval ? "Approval required" : signal ? "Signal required" : "Input required"}
      </h3>
      {intervention.prompt ? (
        <p className="mt-1 text-[11px] leading-relaxed text-muted-foreground">
          {intervention.prompt}
        </p>
      ) : null}
      <textarea
        value={value}
        onChange={(event) => setValue(event.target.value)}
        className="mt-2 min-h-16 w-full resize-y rounded-md border border-hairline bg-background px-2.5 py-2 text-[12px] outline-none focus:border-hairline-strong"
        placeholder={approval || signal ? "Optional note" : "Response"}
      />
      <div className="mt-2 flex justify-end gap-2">
        {approval ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            onClick={() => onSubmit?.({ kind: "reject", value: value.trim() || undefined })}
          >
            <X /> Reject
          </Button>
        ) : null}
        <Button
          type="button"
          size="sm"
          onClick={() =>
            onSubmit?.({
              kind: approval ? "approve" : signal ? "signal" : "resume",
              value: value.trim() || undefined,
            })
          }
        >
          <Check /> {approval ? "Approve" : signal ? "Send signal" : "Resume"}
        </Button>
      </div>
    </section>
  );
}

export function ChatMessageBubble({
  message,
  onApproval,
  onIntervention,
}: { readonly message: ChatMessage } & ChatMessageActions) {
  const streaming = message.status === "streaming";
  const content =
    message.role === "assistant"
      ? sanitizeAssistantMessageContent(message.content)
      : message.content;
  if (message.role === "user") {
    return (
      <div className="ml-auto max-w-[78%] rounded-lg bg-overlay-strong px-3 py-2 text-[12px] leading-relaxed text-foreground whitespace-pre-wrap">
        {content}
      </div>
    );
  }
  return (
    <div className="flex items-start gap-2.5">
      <div
        data-assistant-halo
        className="mt-0.5 flex h-6 w-6 shrink-0 items-center justify-center rounded-full border border-nyx-secondary-400/25 bg-nyx-secondary-400/10 text-nyx-secondary-400"
      >
        <Sparkles className="h-3 w-3" />
      </div>
      <div className="min-w-0 max-w-[min(84%,758px)] flex-1 pt-0.5">
        <ThinkingBlock text={message.thinking ?? ""} streaming={streaming} />
        <RuntimeApprovalCard message={message} onApproval={onApproval} />
        <RunInterventionCard
          key={message.pendingRunIntervention?.key}
          message={message}
          onSubmit={onIntervention}
        />
        <ActivityBlock message={message} />
        {content ? <TextBlock text={content} streaming={streaming} /> : null}
        {streaming && !content ? (
          <div
            data-streaming-dots
            role="status"
            aria-label="Assistant is responding"
            className="flex h-5 items-center gap-1"
          >
            <PulseDot />
            <PulseDot className="[animation-delay:120ms]" />
            <PulseDot className="[animation-delay:240ms]" />
          </div>
        ) : null}
        {message.status === "error" && message.error ? (
          <div className="mt-2 flex items-start gap-2 rounded-lg border border-destructive/25 bg-destructive/[0.05] px-3 py-2 text-[11px] text-destructive">
            <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span>{message.error}</span>
          </div>
        ) : null}
      </div>
    </div>
  );
}

export function ChatMessageEntry({
  message,
  ...actions
}: { readonly message: ChatMessage } & ChatMessageActions) {
  const authorName = message.authorName?.trim() ?? "";
  if (message.role === "user" || message.role === "assistant") {
    return (
      <div className="flex flex-col gap-1">
        {authorName ? (
          <div
            className={cn(
              "text-[10px] text-text-tertiary",
              message.role === "user" ? "self-end" : "ml-8",
            )}
          >
            {authorName}
          </div>
        ) : null}
        <ChatMessageBubble message={message} {...actions} />
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
        <div className="text-[11px] font-medium text-foreground">{displayName}</div>
        {message.thinking ? (
          <p className="mt-1 text-[11px] italic text-text-tertiary">{message.thinking}</p>
        ) : null}
        {message.content ? (
          <div className="mt-1 text-[12px] leading-relaxed text-foreground whitespace-pre-wrap">
            {message.content}
          </div>
        ) : null}
      </div>
    </article>
  );
}

function EmptyState({ children }: { readonly children?: ReactNode }) {
  return (
    <div className="flex flex-1 items-center justify-center px-6 text-center text-[12px] text-text-tertiary">
      {children ?? "Ask NyxID to help with services, access, and account operations."}
    </div>
  );
}

export function ChatMessageList({
  session,
  bottomInset,
  emptyDescription,
  footer,
  ...actions
}: {
  readonly session: ChatSessionState | null;
  readonly bottomInset: number;
  readonly emptyDescription?: ReactNode;
  readonly footer?: ReactNode;
} & ChatMessageActions) {
  const [detectedMessageId, setDetectedMessageId] = useState<string>();
  const messages = session?.messages ?? [];
  const terminalAssistant = messages.at(-1);
  const emptyTerminal = Boolean(
    session &&
      session.status !== "streaming" &&
      session.status !== "draft" &&
      session.status !== "stopped" &&
      terminalAssistant?.role === "assistant" &&
      !terminalAssistant.content.trim() &&
      !terminalAssistant.error &&
      !(terminalAssistant.steps?.length || terminalAssistant.toolCalls?.length),
  );
  useEffect(() => {
    if (!emptyTerminal) return;
    const messageId = terminalAssistant?.id;
    const timer = setTimeout(() => setDetectedMessageId(messageId), 700);
    return () => clearTimeout(timer);
  }, [emptyTerminal, terminalAssistant?.id]);
  const emptyTurnDetected =
    emptyTerminal && detectedMessageId === terminalAssistant?.id;

  if (!messages.length) return <EmptyState>{emptyDescription}</EmptyState>;
  return (
    <div className="min-h-0 flex-1 overflow-y-auto px-4 sm:px-6">
      <div
        className="mx-auto flex w-full max-w-[758px] flex-col gap-4 pt-6"
        style={{ paddingBottom: Math.max(bottomInset + 24, 96) }}
      >
        {messages.map((message) => (
          <ChatMessageEntry key={message.id} message={message} {...actions} />
        ))}
        {footer}
        {emptyTurnDetected ? (
          <span data-empty-turn-error className="sr-only" aria-hidden="true" />
        ) : null}
      </div>
    </div>
  );
}
