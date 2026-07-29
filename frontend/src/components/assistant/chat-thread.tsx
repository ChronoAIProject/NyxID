import { Fragment, useCallback, useEffect, useRef } from "react";
import { AlertCircle } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ArtifactBlock } from "@/components/assistant/blocks/artifact-block";
import { ConnectCard } from "@/components/assistant/blocks/connect-card";
import { RunCard } from "@/components/assistant/blocks/run-card";
import { TextBlock } from "@/components/assistant/blocks/text-block";
import { useFadingPresence } from "@/hooks/use-fading-presence";
import haloSheet from "@/assets/halo-sheet.webp";
import { formatClockTime } from "@/lib/utils";
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
 * A text block with nothing in it.
 *
 * The transport always leads an assistant message with a text block, even when
 * the message opens with a connect card, so the live and reloaded block lists
 * stay identical (that convergence is what lets cards survive a reload). Such a
 * block has no content to show and must not occupy a row.
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
    case "artifact":
      return <ArtifactBlock block={typed} />;
    default:
      return <UnsupportedContent />;
  }
}

/**
 * Assistant identity mark (no bubble — the answer reads as the assistant
 * "speaking" directly). The brand mark stays so the reader can tell turns
 * apart; the user side drops its icon for a cleaner asymmetric layout.
 *
 * Placement is container-query driven (see `ASSISTANT_ROW`): while the chat
 * has room it sits in a left gutter beside the answer; once the chat window
 * narrows past the thread's natural width it stacks on top instead.
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
    <div className="flex shrink-0 items-center gap-2 @min-[680px]:flex-col @min-[680px]:items-start @min-[680px]:gap-1">
      <span className="relative flex h-[18px] w-[18px] shrink-0 items-center justify-center overflow-visible">
        {showHalo ? (
          <span
            aria-hidden="true"
            data-assistant-halo
            className={`assistant-halo ${halo.visible ? "assistant-halo--visible" : ""}`}
          >
            <span
              className="assistant-halo-sprite"
              style={{ backgroundImage: `url(${haloSheet})` }}
            />
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

// Icon-left when the chat window is at least as wide as the thread; icon-on-top
// once it is squeezed narrower. Queried against the thread container, not the
// viewport, since the assistant surface sits beside a sidebar.
const ASSISTANT_ROW =
  "group flex flex-col gap-1.5 @min-[680px]:flex-row @min-[680px]:items-start @min-[680px]:gap-3";

type MessageGroup = {
  readonly role: AssistantMessage["role"];
  readonly messages: readonly AssistantMessage[];
};

/**
 * Collapse consecutive same-role messages into one group. Aevatar streams a
 * single turn as several messages (text, then a tool run, then more text);
 * they belong to one "voice" and must render under a single identity mark, not
 * repeat the icon per message.
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

// Standalone caret for the brief window after a turn starts but before its
// first block arrives (the inline caret in TextBlock covers streaming text).
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

// Slack, in px, within which the thread still counts as "following" the tail.
// Below it the reader has deliberately scrolled up and must not be yanked back
// while the assistant streams.
const FOLLOW_THRESHOLD = 48;

export function ChatThread({
  messages,
  thinking = false,
  streaming = false,
  bottomInset = 0,
  onDecideApproval,
}: {
  readonly messages: readonly AssistantMessage[];
  /**
   * Turn is running but no assistant content has arrived yet. Aevatar can
   * take seconds before its first frame; without this the thread reads as
   * dead between send and first answer.
   */
  readonly thinking?: boolean;
  /**
   * Turn is running and the assistant is the current speaker — drives the
   * blinking caret at the end of the latest assistant group so streaming
   * reads as live typing rather than a frozen partial answer.
   */
  readonly streaming?: boolean;
  /**
   * Height in px of the composer floating over the thread. Turns are allowed to
   * scroll behind it (ChatGPT-style), so the thread reserves this much room at
   * the tail and dissolves into it over the same distance.
   */
  readonly bottomInset?: number;
  readonly onDecideApproval: (
    blockId: string,
    approved: boolean,
  ) => Promise<void>;
}) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const thinkingPresence = useFadingPresence(thinking, 500);
  const following = useRef(true);
  const lastSentId = useRef<string | undefined>(undefined);

  // Nothing references the halo strip until a thinking state mounts, so the
  // fetch would otherwise start at the exact moment the halo needs to be
  // decoded already — and the first wait of a session would show a blank gutter
  // instead. Warm it when the thread opens rather than preloading it app-wide.
  useEffect(() => {
    new Image().src = haloSheet;
  }, []);

  const handleScroll = useCallback(() => {
    const element = scrollRef.current;
    if (!element) return;
    following.current =
      element.scrollHeight - element.scrollTop - element.clientHeight <
      FOLLOW_THRESHOLD;
  }, []);

  useEffect(() => {
    const element = scrollRef.current;
    if (!element) return;
    // Sending always pulls the view back to the tail; only assistant streaming
    // respects a deliberate scroll-up. Keyed on the message id so the poll loop
    // re-rendering the same turn doesn't keep yanking a reader back down.
    const latest = messages.at(-1);
    if (latest?.role === "user" && latest.id !== lastSentId.current) {
      lastSentId.current = latest.id;
      following.current = true;
    }
    if (following.current) element.scrollTop = element.scrollHeight;
  }, [messages, thinking, streaming, bottomInset]);

  const groups = groupMessages(messages);

  // Opaque down to the top of the composer, then dissolved to nothing by the
  // bottom edge — content passing behind the composer fades out instead of
  // being clipped by a hard line.
  const fadeMask = `linear-gradient(to bottom, #000 0, #000 calc(100% - ${String(bottomInset)}px), transparent 100%)`;

  if (messages.length === 0) {
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
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        onScroll={handleScroll}
        className="@container min-h-0 flex-1 overflow-y-auto overscroll-contain"
        style={{ maskImage: fadeMask, WebkitMaskImage: fadeMask }}
      >
        <div
          className="mx-auto flex w-full max-w-[680px] flex-col gap-6 px-4 py-6 sm:px-6 sm:py-8"
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
                  className="group flex flex-col items-end gap-1"
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
                              {renderBlock(block, onDecideApproval)}
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

            // One identity for the whole group; every message's blocks (text,
            // tool runs, cards) stack in the shared content column below it.
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
                          // A message that opens with a connect card still
                          // carries an empty leading text block, so the live
                          // and reloaded block lists stay identical. Rendering
                          // its wrapper would add a `space-y-3` gap above the
                          // card for no content.
                          if (isEmptyTextBlock(block)) return null;
                          return (
                            <div key={`${blockId(block)}-${String(index)}`}>
                              {renderBlock(
                                block,
                                onDecideApproval,
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
                  {awaitingFirstBlock ? <StreamingCaret /> : null}
                </div>
              </article>
            );
          })}
          {thinkingPresence.present && !streaming ? (
            <ThinkingRow loading={thinking} />
          ) : null}
        </div>
      </div>
    </div>
  );
}
