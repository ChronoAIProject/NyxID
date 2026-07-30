import {
  Fragment,
  useCallback,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { AlertCircle } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ActionCard } from "@/components/assistant/blocks/action-card";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ArtifactBlock } from "@/components/assistant/blocks/artifact-block";
import { ConnectCard } from "@/components/assistant/blocks/connect-card";
import { RunCard } from "@/components/assistant/blocks/run-card";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { useFadingPresence } from "@/hooks/use-fading-presence";
import { formatClockTime } from "@/lib/utils";
import type { ActionReport } from "@/schemas/assistant-actions";
import type { AssistantMessage, ContentBlock } from "@/types/assistant";

function UnsupportedContent() {
  return (
    <div className="flex items-center gap-2 rounded-lg border border-border bg-overlay px-3 py-2 text-[11px] text-muted-foreground">
      <AlertCircle className="h-3.5 w-3.5 shrink-0 text-text-tertiary" />
      Unsupported assistant content
    </div>
  );
}

function blockId(block: unknown): string {
  if (
    typeof block === "object" &&
    block !== null &&
    "block_id" in block &&
    typeof block.block_id === "string"
  ) {
    return block.block_id;
  }
  return "unsupported-block";
}

/**
 * The transport can lead an assistant message with an empty text block before
 * a card. It keeps live and reloaded block lists identical, but has no row to
 * render in the transcript.
 */
function isEmptyTextBlock(block: unknown): boolean {
  return (
    typeof block === "object" &&
    block !== null &&
    "type" in block &&
    block.type === "text" &&
    "text" in block &&
    typeof block.text === "string" &&
    block.text.trim() === ""
  );
}

function isTextBlock(block: unknown): boolean {
  return (
    typeof block === "object" &&
    block !== null &&
    "type" in block &&
    (block as { type: unknown }).type === "text"
  );
}

function renderBlock(
  block: unknown,
  onDecideApproval: (blockId: string, approved: boolean) => Promise<void>,
  onActionProgress: (blockId: string, inProgress: boolean) => void,
  onResolveAction: (report: ActionReport) => Promise<void>,
  streaming = false,
) {
  if (typeof block !== "object" || block === null || !("type" in block)) {
    return <UnsupportedContent />;
  }
  const typed = block as ContentBlock;
  switch (typed.type) {
    case "text":
      return <TextBlock text={typed.text} streaming={streaming} />;
    case "connect_card":
      return <ConnectCard block={typed} />;
    case "run":
      return <RunCard block={typed} />;
    case "approval_card":
      return (
        <ApprovalCard
          block={typed}
          onDecide={(approved) => onDecideApproval(typed.block_id, approved)}
        />
      );
    case "action_card":
      return (
        <ActionCard
          block={typed}
          onProgress={onActionProgress}
          onResolve={onResolveAction}
        />
      );
    case "artifact":
      return <ArtifactBlock block={typed} />;
    default:
      return <UnsupportedContent />;
  }
}

/**
 * The brand mark occupies a fixed left gutter. The transcript, cards, user
 * bubbles, and composer all use the adjacent content column at every width.
 */
function AssistantIdentity({
  time,
  loading = false,
  allowHaloExit = true,
}: {
  readonly time: string;
  readonly loading?: boolean;
  readonly allowHaloExit?: boolean;
}) {
  const halo = useFadingPresence(loading, 500);
  const showHalo = halo.present && (loading || allowHaloExit);

  return (
    <div className="flex shrink-0 flex-col items-start gap-1">
      <span className="relative flex h-[18px] w-[18px] shrink-0 items-center justify-center overflow-visible">
        {showHalo ? (
          <span
            aria-hidden="true"
            data-assistant-halo
            className={`assistant-halo ${halo.visible ? "assistant-halo--visible" : ""}`}
          >
            <span className="assistant-halo-ring animate-halo-spin" />
            <span className="assistant-halo-ring assistant-halo-ring-reverse animate-halo-spin-reverse" />
          </span>
        ) : null}
        <span className="relative z-10 flex h-[18px] w-[18px] items-center justify-center rounded-md border border-nyx-secondary-400/20 bg-nyx-secondary-400/[0.06]">
          <NyxidIcon className="h-[11px] w-[11px]" />
        </span>
      </span>
      {time ? (
        <span className="font-mono text-[10px] text-text-tertiary tabular-nums opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
          {time}
        </span>
      ) : null}
    </div>
  );
}

const ASSISTANT_ROW =
  "group grid grid-cols-[18px_minmax(0,1fr)] items-start gap-x-3";

type MessageGroup = {
  readonly role: AssistantMessage["role"];
  readonly messages: readonly AssistantMessage[];
};

/**
 * Collapse consecutive same-role messages into one group. Aevatar streams a
 * single turn as several messages, and they belong under one identity mark.
 */
function groupMessages(messages: readonly AssistantMessage[]): MessageGroup[] {
  const groups: {
    role: AssistantMessage["role"];
    messages: AssistantMessage[];
  }[] = [];
  for (const message of messages) {
    const last = groups.at(-1);
    if (last && last.role === message.role) last.messages.push(message);
    else groups.push({ role: message.role, messages: [message] });
  }
  return groups;
}

function StreamingCaret() {
  return (
    <span
      aria-hidden
      className="inline-block h-4 w-[2px] animate-pulse rounded-full bg-nyx-secondary-400 align-middle"
    />
  );
}

function ThinkingRow({ loading }: { readonly loading: boolean }) {
  return (
    <article
      className={ASSISTANT_ROW}
      role={loading ? "status" : undefined}
      aria-label={loading ? "Assistant is thinking" : undefined}
      aria-hidden={loading ? undefined : true}
    >
      <AssistantIdentity time="" loading={loading} />
      <div className="min-h-[18px] min-w-0 flex-1" />
    </article>
  );
}

// Slack within which the thread still counts as following the tail.
const FOLLOW_THRESHOLD = 48;

export function ChatThread({
  messages,
  thinking = false,
  streaming = false,
  bottomInset = 0,
  onDecideApproval,
  onActionProgress = () => undefined,
  onResolveAction = async () => undefined,
}: {
  readonly messages: readonly AssistantMessage[];
  readonly thinking?: boolean;
  readonly streaming?: boolean;
  /** Height of the composer floating over the transcript. */
  readonly bottomInset?: number;
  readonly onDecideApproval: (
    blockId: string,
    approved: boolean,
  ) => Promise<void>;
  readonly onActionProgress?: (blockId: string, inProgress: boolean) => void;
  readonly onResolveAction?: (report: ActionReport) => Promise<void>;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrolledFromTop, setScrolledFromTop] = useState(false);
  const thinkingPresence = useFadingPresence(thinking, 500);
  const following = useRef(true);
  const lastSentId = useRef<string | undefined>(undefined);
  const hasMessages = messages.length > 0;

  const syncScrollPosition = useCallback((element: HTMLDivElement) => {
    following.current =
      element.scrollHeight - element.scrollTop - element.clientHeight <
      FOLLOW_THRESHOLD;
    setScrolledFromTop(element.scrollTop > 1);
  }, []);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    // Sending always pulls the view back to the tail; assistant streaming still
    // respects a reader who deliberately scrolled upward.
    const latest = messages.at(-1);
    if (latest?.role === "user" && latest.id !== lastSentId.current) {
      lastSentId.current = latest.id;
      following.current = true;
    }
    if (following.current) element.scrollTop = element.scrollHeight;
    setScrolledFromTop(element.scrollTop > 1);
  }, [messages, thinking, streaming, bottomInset]);

  useLayoutEffect(() => {
    const root = rootRef.current;
    const scroll = scrollRef.current;
    const chatSurface = root?.parentElement;
    if (!scroll || !chatSurface) return;

    const previousWidth = chatSurface.style.getPropertyValue(
      "--assistant-scrollbar-width",
    );
    const restoreScrollbarWidth = () => {
      if (previousWidth) {
        chatSurface.style.setProperty(
          "--assistant-scrollbar-width",
          previousWidth,
        );
      } else {
        chatSurface.style.removeProperty("--assistant-scrollbar-width");
      }
    };
    const syncScrollbarWidth = () => {
      const width = Math.max(0, scroll.offsetWidth - scroll.clientWidth);
      chatSurface.style.setProperty(
        "--assistant-scrollbar-width",
        `${String(width)}px`,
      );
    };

    syncScrollbarWidth();
    if (typeof ResizeObserver === "undefined") {
      return restoreScrollbarWidth;
    }

    const observer = new ResizeObserver(syncScrollbarWidth);
    observer.observe(scroll);
    return () => {
      observer.disconnect();
      restoreScrollbarWidth();
    };
  }, [hasMessages]);

  const groups = groupMessages(messages);

  // Content passes behind the floating composer and dissolves into its top.
  const fadeMask = `linear-gradient(to bottom, #000 0, #000 calc(100% - ${String(bottomInset)}px), transparent 100%)`;

  if (!hasMessages) {
    return (
      <div
        className="flex flex-1 items-center justify-center px-6 text-center"
        style={{ paddingBottom: `${String(bottomInset)}px` }}
      >
        <div>
          <div className="mx-auto flex h-10 w-10 items-center justify-center rounded-xl border border-nyx-secondary-400/20 bg-nyx-secondary-400/[0.06]">
            <NyxidIcon className="h-5 w-5" />
          </div>
          <p className="mt-3 text-[13px] font-medium text-foreground">
            Start a new conversation
          </p>
          <p className="mt-1 text-[11px] text-muted-foreground">
            Ask NyxID to work with your connected services.
          </p>
        </div>
      </div>
    );
  }

  return (
    <div ref={rootRef} className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        onScroll={(event) => syncScrollPosition(event.currentTarget)}
        className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain"
        style={{ maskImage: fadeMask, WebkitMaskImage: fadeMask }}
      >
        <div
          className="mx-auto flex w-full max-w-[758px] flex-col gap-6 px-4 py-6 sm:px-6 sm:py-8"
          style={{ paddingBottom: `calc(${String(bottomInset)}px + 1.5rem)` }}
        >
          {groups.map((group, groupIndex) => {
            const first = group.messages[0];
            const time = formatClockTime(first?.created_at);
            const isLastGroup = groupIndex === groups.length - 1;

            if (group.role === "user") {
              return (
                <article
                  key={first?.id}
                  className="group ml-[30px] flex flex-col items-end gap-1"
                >
                  <span className="sr-only">You</span>
                  {group.messages.map((message) => (
                    <div
                      key={message.id}
                      className="max-w-[85%] rounded-xl rounded-br-[2px] bg-overlay-strong px-3.5 py-2"
                    >
                      {message.schema_version !== 1 ? (
                        <UnsupportedContent />
                      ) : (
                        <div className="space-y-3">
                          {(message.blocks as unknown[]).map((block, index) => (
                            <div key={`${blockId(block)}-${String(index)}`}>
                              {renderBlock(
                                block,
                                onDecideApproval,
                                onActionProgress,
                                onResolveAction,
                              )}
                            </div>
                          ))}
                        </div>
                      )}
                    </div>
                  ))}
                  {time ? (
                    <span className="px-1 font-mono text-[10px] text-text-tertiary tabular-nums opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
                      {time}
                    </span>
                  ) : null}
                </article>
              );
            }

            const streamingGroup = isLastGroup && streaming;
            const lastMessage = group.messages.at(-1);
            const awaitingFirstBlock =
              streamingGroup && (lastMessage?.blocks.length ?? 0) === 0;

            return (
              <article key={first?.id} className={ASSISTANT_ROW}>
                <AssistantIdentity
                  time={time}
                  loading={streamingGroup}
                  allowHaloExit={!thinking && !streaming}
                />
                <div className="min-w-0 flex-1 space-y-3">
                  {group.messages.map((message) => {
                    if (message.schema_version !== 1) {
                      return <UnsupportedContent key={message.id} />;
                    }
                    const isLastMessage = message === lastMessage;
                    return (
                      <Fragment key={message.id}>
                        {(message.blocks as unknown[]).map((block, index) => {
                          const isLastBlock =
                            isLastMessage &&
                            index === message.blocks.length - 1;
                          if (isEmptyTextBlock(block)) return null;
                          return (
                            <div
                              key={`${blockId(block)}-${String(index)}`}
                              className={
                                isTextBlock(block) ? "pl-[7px]" : undefined
                              }
                            >
                              {renderBlock(
                                block,
                                onDecideApproval,
                                onActionProgress,
                                onResolveAction,
                                streamingGroup &&
                                  isLastBlock &&
                                  isTextBlock(block),
                              )}
                            </div>
                          );
                        })}
                      </Fragment>
                    );
                  })}
                  {awaitingFirstBlock ? (
                    <span className="ml-[7px]">
                      <StreamingCaret />
                    </span>
                  ) : null}
                </div>
              </article>
            );
          })}
          {thinkingPresence.present && !streaming ? (
            <ThinkingRow loading={thinking} />
          ) : null}
        </div>
      </div>
      <div
        aria-hidden
        className={`pointer-events-none absolute inset-x-0 top-0 h-7 bg-gradient-to-b from-background via-background/70 to-transparent transition-opacity duration-150 motion-reduce:transition-none ${
          scrolledFromTop ? "opacity-100" : "opacity-0"
        }`}
      />
    </div>
  );
}
