import { useMemo } from "react";
import { FileJson2 } from "lucide-react";
import { ChatMessageEntry } from "@/components/assistant/chat-message";
import {
  createChatActorProjection,
  decodeActorFrame,
  reduceActorFrame,
  type ChatActorProjection,
} from "@/lib/assistant/chat-actor-state";
import { buildAssistantMessagePatch } from "@/lib/assistant/chat-session-state";
import type { ChatMessage } from "@/lib/assistant/chat-types";
import {
  applyRuntimeEvent,
  createRuntimeEventAccumulator,
} from "@/lib/assistant/runtime-event-semantics";
import {
  normalizeBackendSseFrame,
  SsePayloadDecoder,
} from "@/lib/assistant/sse-frame-normalizer";
import type { AssistantWireLogExchange } from "@/schemas/assistant-wire-log";

interface AssistantWireReplayProjection {
  readonly actorFacts: readonly unknown[];
  readonly actorProjection: ChatActorProjection;
  readonly message: ChatMessage | null;
  readonly partial: boolean;
}

function captureText(exchange: AssistantWireLogExchange): string {
  const capture = exchange.capture;
  if (!capture || capture.state === "evicted") return "";
  if (capture.body) return capture.body.text;
  return capture.sse?.lines
    .map((line) => `${line.text}${line.ending}`)
    .join("") ?? "";
}

function capturedPayloads(text: string): string[] {
  const decoder = new SsePayloadDecoder();
  return [...decoder.push(new TextEncoder().encode(text)), ...decoder.finish()];
}

function captureIsPartial(exchange: AssistantWireLogExchange): boolean {
  const capture = exchange.capture;
  if (!capture || capture.state !== "settled") return true;
  return (
    capture.wireOutcome !== "complete" ||
    Boolean(capture.body?.truncated || capture.sse?.truncated)
  );
}

function isNyxIdDiagnosticFact(raw: unknown): boolean {
  if (!raw || typeof raw !== "object" || Array.isArray(raw)) return false;
  const custom = (raw as { custom?: unknown }).custom;
  if (!custom || typeof custom !== "object" || Array.isArray(custom)) {
    return false;
  }
  const name = (custom as { name?: unknown }).name;
  return typeof name === "string" && name.startsWith("nyxid.");
}

// Exported for deterministic replay tests; the adjacent component is the only UI consumer.
// eslint-disable-next-line react-refresh/only-export-components
export function replayAssistantWireExchange(
  exchange: AssistantWireLogExchange,
): AssistantWireReplayProjection | null {
  const text = captureText(exchange);
  if (!text) return null;
  const accumulator = createRuntimeEventAccumulator();
  let actorProjection = createChatActorProjection(exchange.conversationId);
  const actorFacts: unknown[] = [];
  let sawFrame = false;
  let terminal = false;

  for (const payload of capturedPayloads(text)) {
    const data = payload.trim();
    if (!data || data === "[DONE]") continue;
    let raw: unknown;
    try {
      raw = JSON.parse(data) as unknown;
    } catch {
      continue;
    }
    sawFrame = true;
    try {
      const actorFrame = decodeActorFrame(raw);
      if (actorFrame.type !== "ignored") {
        actorFacts.push(raw);
        actorProjection = reduceActorFrame(actorProjection, actorFrame);
      } else if (isNyxIdDiagnosticFact(raw)) {
        actorFacts.push(raw);
      }
    } catch {
      actorFacts.push(raw);
    }
    const event = normalizeBackendSseFrame(raw);
    if (!event) continue;
    applyRuntimeEvent(accumulator, event);
    terminal ||= ["RUN_FINISHED", "RUN_ERROR", "RUN_STOPPED"].includes(
      String(event.type),
    );
  }
  if (!sawFrame) return null;
  const status = accumulator.errorText ? "error" : terminal ? "complete" : "streaming";
  const patch = buildAssistantMessagePatch(accumulator, status);
  const hasMessage = Boolean(
    patch.content ||
      patch.error ||
      patch.thinking ||
      patch.steps?.length ||
      patch.toolCalls?.length,
  );
  return {
    actorFacts,
    actorProjection,
    message: hasMessage
      ? {
          content: "",
          id: `wire-replay:${exchange.id}`,
          role: "assistant",
          status,
          timestamp: exchange.ts,
          ...patch,
        }
      : null,
    partial: captureIsPartial(exchange) || !terminal,
  };
}

export function AssistantWireReplayView({
  exchange,
}: {
  readonly exchange: AssistantWireLogExchange;
}) {
  const replay = useMemo(
    () => replayAssistantWireExchange(exchange),
    [exchange],
  );
  if (!replay) return null;

  return (
    <div className="space-y-3" data-testid="wire-replay-view">
      {replay.partial ? (
        <p className="rounded-lg border border-warning/30 bg-warning/10 px-3 py-2 text-[11px] text-warning">
          Partial replay: the delivered capture was truncated or ended without a
          clean terminal frame.
        </p>
      ) : null}
      <p className="text-[11px] leading-5 text-text-tertiary">
        Message content uses the production chat renderer. Actor facts are
        diagnostic only and cannot be acted on here.
      </p>
      {replay.message ? (
        <ChatMessageEntry message={replay.message} interactiveCards={false} />
      ) : replay.actorFacts.length === 0 ? (
        <p className="py-5 text-center text-[11px] text-text-tertiary">
          No renderable chat content in this capture.
        </p>
      ) : null}
      {replay.actorFacts.length ? (
        <section
          aria-label="Actor facts (diagnostic only)"
          className="overflow-hidden rounded-lg border border-border/70 bg-background/50"
        >
          <div className="flex items-center gap-2 border-b border-border/60 px-3 py-2">
            <FileJson2 className="h-3.5 w-3.5 text-text-tertiary" />
            <span className="text-[11px] font-medium text-foreground">
              Actor facts
            </span>
            <span className="ml-auto text-[10px] text-text-tertiary">
              Diagnostic only
            </span>
          </div>
          <pre className="max-h-56 overflow-auto whitespace-pre-wrap break-all p-3 font-mono text-[10px] leading-5 text-muted-foreground">
            {JSON.stringify(replay.actorFacts, null, 2)}
          </pre>
        </section>
      ) : null}
    </div>
  );
}
