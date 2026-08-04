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
import haloSheet from "@/assets/halo-sheet.webp";
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
  onBlockAction: (blockId: string, note: string) => void,
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
          onBlock={onBlockAction}
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

const ASSISTANT_ROW =
  "group grid grid-cols-[18px_minmax(0,1fr)] items-start gap-x-3";

type MessageGroup = {
  readonly role: AssistantMessage["role"];
  readonly messages: readonly AssistantMessage[];
};

/**
 * Whether a group has anything the reader can actually see. Block COUNT is not
 * the test: a turn that opens with a connect card carries an empty leading text
 * block, and a text block that has started but produced no characters is also
 * present-but-blank. Both must still count as "nothing printed yet", or the
 * dots would vanish before any answer appeared.
 *
 * An unsupported-schema message renders a visible shell, so it counts.
 */
function hasPrintableContent(messages: readonly AssistantMessage[]): boolean {
  return messages.some(
    (message) =>
      message.schema_version !== 1 ||
      (message.blocks as unknown[]).some((block) => !isEmptyTextBlock(block)),
  );
}

/**
 * True once `condition` has held continuously for `delayMs`.
 *
 * The turn-status event and the transcript projection land in either order, so
 * a turn that DID answer can look content-free for a frame or two. Used to gate
 * the empty-turn error: a wrong "there was an error" is far worse than showing
 * a right one a beat late.
 */
function useSettled(condition: boolean, delayMs: number): boolean {
  const [settled, setSettled] = useState(false);
  const [armedFor, setArmedFor] = useState(condition);

  // Render-phase reset, not an effect. Clearing in an effect leaves `settled`
  // true across a false -> true flip that happens inside one macrotask: the
  // rearm's cleanup cancels the pending reset before it ever runs, and the new
  // episode then reports settled immediately instead of waiting its turn.
  if (armedFor !== condition) {
    setArmedFor(condition);
    setSettled(false);
  }

  useEffect(() => {
    if (!condition) return;
    const timer = window.setTimeout(() => setSettled(true), delayMs);
    return () => window.clearTimeout(timer);
  }, [condition, delayMs]);

  return condition && settled;
}

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

/**
 * Placeholder for the window between a turn starting and its first printable
 * content: it sits exactly where the answer will appear, so the dots are
 * replaced by text rather than pushing it around. The halo says a turn is
 * running; these say the answer itself is on its way.
 *
 * The inline caret in TextBlock takes over once characters exist. This slot
 * briefly held a standalone caret instead; the dots replace it, because a
 * caret alone is thinner than the gap it has to explain.
 *
 * Three dots lighting in turn — see `.assistant-streaming-dot` in app.css.
 * This is deliberately the plainest thing that can say "still coming": the
 * loader it replaces was a four-ball Newton's cradle whose swing, gravity
 * easing and directional exit sweep drew the eye to the loader instead of to
 * the answer arriving behind it.
 */
const THINKING_DOTS = 3;
// The exit is a 200ms fade with no travel behind it; the margin is deliberate
// slack so the final frame is painted rather than raced against the unmount.
const DOTS_EXIT_MS = 220;

function StreamingDots({
  visible,
  live = false,
}: {
  readonly visible: boolean;
  readonly live?: boolean;
}) {
  const rootRef = useRef<HTMLSpanElement>(null);
  const lastVisibleRect = useRef<DOMRect | undefined>(undefined);
  const [present, setPresent] = useState(visible);

  if (visible && !present) setPresent(true);

  useLayoutEffect(() => {
    if (visible && rootRef.current) {
      lastVisibleRect.current = rootRef.current.getBoundingClientRect();
    }
  });

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (visible || !present || !root) return;

    // The dots sit in the flow where the answer lands, so a fade that stayed
    // in flow would hold that row open for its whole duration and then let it
    // collapse under the arriving text. Pinning the last measured box takes
    // the fade out of layout entirely: content moves up immediately and the
    // dots dissolve over the top of nothing.
    const rect = lastVisibleRect.current ?? root.getBoundingClientRect();
    root.classList.remove("relative");
    root.classList.add(
      "assistant-streaming-dots--leaving",
      "pointer-events-none",
      "fixed",
    );
    root.style.left = `${String(rect.left)}px`;
    root.style.top = `${String(rect.top)}px`;
    root.style.width = `${String(rect.width)}px`;
    root.style.height = `${String(rect.height)}px`;

    const exitTimer = window.setTimeout(() => {
      setPresent(false);
    }, DOTS_EXIT_MS);
    return () => {
      window.clearTimeout(exitTimer);
      root.classList.add("relative");
      root.classList.remove(
        "assistant-streaming-dots--leaving",
        "pointer-events-none",
        "fixed",
      );
      root.style.removeProperty("left");
      root.style.removeProperty("top");
      root.style.removeProperty("width");
      root.style.removeProperty("height");
    };
  }, [present, visible]);

  if (!present) return null;

  return (
    <span
      ref={rootRef}
      data-streaming-dots
      // Three 6px dots on an 11px pitch — a 28px group, close enough to the
      // 30px the four-ball loader occupied that nothing around it re-flows.
      // The 5px gap is nearly a full dot of air: these are three separate
      // marks taking turns, not a chain in contact, and the spacing has to say
      // so before the animation does.
      className="assistant-streaming-dots relative ml-[7px] flex h-[18px] w-max items-center gap-[5px]"
      // The standalone thinking row is itself a live region, so its dots stay
      // decorative. Dots inside an opened-but-still-empty assistant message
      // have no such wrapper — without their own role that whole pre-content
      // state is silent to a screen reader.
      role={visible && live ? "status" : undefined}
      aria-label={visible && live ? "Assistant is answering" : undefined}
      aria-hidden={!visible || !live ? true : undefined}
    >
      {Array.from({ length: THINKING_DOTS }, (_, index) => (
        <span
          // The dots have no independent meaning; the parent owns the status.
          aria-hidden="true"
          // Opacity is animated, so the resting value lives in app.css beside
          // the keyframes rather than as a utility here that the animation
          // would only override.
          className="assistant-streaming-dot h-[6px] w-[6px] rounded-full bg-muted-foreground"
          key={index}
        />
      ))}
    </span>
  );
}

function ThinkingRow({
  loading,
  overlay,
}: {
  readonly loading: boolean;
  readonly overlay: boolean;
}) {
  const active = loading && !overlay;

  return (
    <article
      className={`${ASSISTANT_ROW} ${overlay ? "pointer-events-none absolute" : "relative"}`}
      role={active ? "status" : undefined}
      aria-label={active ? "Assistant is thinking" : undefined}
      aria-hidden={active ? undefined : true}
    >
      {overlay ? <span aria-hidden /> : <AssistantIdentity time="" loading={loading} />}
      <div className="min-h-[18px] min-w-0 flex-1">
        <StreamingDots visible={active} />
      </div>
    </article>
  );
}

// How long a content-free closed turn must stay content-free before it is
// called an error. Longer than the thinking row's 500 ms exit fade, so the two
// never overlap.
const EMPTY_TURN_GRACE_MS = 700;

// Slack, in px, within which the thread still counts as "following" the tail.
// Below it the reader has deliberately scrolled up and must not be yanked back
// while the assistant streams.
const FOLLOW_THRESHOLD = 48;

export function ChatThread({
  messages,
  thinking = false,
  streaming = false,
  turnEnded = false,
  turnPrinted,
  transcriptSettling = false,
  bottomInset = 0,
  onDecideApproval,
  onActionProgress = () => undefined,
  onBlockAction = () => undefined,
  onResolveAction = async () => undefined,
}: {
  readonly messages: readonly AssistantMessage[];
  readonly thinking?: boolean;
  readonly streaming?: boolean;
  /**
   * The latest turn reached a terminal state on its own (completed or failed —
   * NOT cancelled, which is the reader pressing Stop and not an error). Paired
   * with a content-free tail this is what distinguishes "the stream closed
   * having printed nothing" from "no turn has run yet".
   */
  readonly turnEnded?: boolean;
  /**
   * Whether the CURRENT stream episode has printed anything, as reported by the
   * pump serving it. Authoritative when given, because the transcript cannot
   * answer it: an approval continuation is appended to the previous turn's
   * assistant group, so earlier content is indistinguishable from its own.
   * Undefined after a reload, where the transcript is all there is.
   */
  readonly turnPrinted?: boolean;
  /**
   * A transcript read is in flight. The turn's terminal status and its
   * transcript arrive independently, so this is what keeps a slow-projecting
   * answer from being reported as a turn that printed nothing.
   */
  readonly transcriptSettling?: boolean;
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
  readonly onActionProgress?: (blockId: string, inProgress: boolean) => void;
  readonly onBlockAction?: (blockId: string, note: string) => void;
  readonly onResolveAction?: (report: ActionReport) => Promise<void>;
}) {
  const rootRef = useRef<HTMLDivElement>(null);
  const scrollRef = useRef<HTMLDivElement>(null);
  const [scrolledFromTop, setScrolledFromTop] = useState(false);
  const thinkingPresence = useFadingPresence(thinking, 500);
  const following = useRef(true);
  const lastSentId = useRef<string | undefined>(undefined);
  const hasMessages = messages.length > 0;

  // Nothing references the halo strip until a thinking state mounts, so the
  // fetch would otherwise start at the exact moment the halo needs to be
  // decoded already — and the first wait of a session would show a blank gutter
  // instead. Warm it when the thread opens rather than preloading it app-wide.
  useEffect(() => {
    new Image().src = haloSheet;
  }, []);

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

  // A turn that closed having printed nothing: either it never produced an
  // assistant message at all (tail is still the reader's own), or it produced
  // one that never got past empty blocks. Settled over a beat so the two
  // out-of-order arrivals — turn status and transcript projection — cannot
  // flag a turn that did answer.
  //
  // DETECTION ONLY — deliberately not rendered. Production traces showed the
  // reply regularly exists upstream and materializes into the transcript
  // moments later (the reconciler projects it in with no reload), so an
  // on-screen "didn't reply" here is a false negative more often than not.
  // The durable record of the empty stream is the transport's wire-log
  // telemetry (transportOutcome + printable-event counts); this attribute
  // keeps the condition observable for tests and DOM inspection until the
  // refined failure presentation ships (docs/plans/newchat-followup-fix.md,
  // deferred W4).
  const tail = groups.at(-1);
  // `turnPrinted` when the pump can tell us; the transcript only as the
  // after-a-reload fallback, where it is the sole record that exists.
  const tailAnswered =
    turnPrinted ??
    (tail?.role === "assistant" && hasPrintableContent(tail.messages));
  const emptyTurnDetected = useSettled(
    turnEnded &&
      !thinking &&
      !streaming &&
      !tailAnswered &&
      // A read still in flight is the case the grace period is guessing at.
      // When the caller can tell us outright, don't guess: an answer that is
      // simply slow to project must never be flagged.
      !transcriptSettling,
    EMPTY_TURN_GRACE_MS,
  );

  // Deliberately the RAW terminal state, not the settled detection. A turn
  // that has closed is never "no conversation yet", and gating the screen on
  // the delayed detection would show "start a new conversation" all through
  // the grace period — or forever, if something keeps the settle condition
  // suppressed.
  const turnHasRun = thinking || streaming || turnEnded;

  // Opaque down to the top of the composer, then dissolved to nothing by the
  // bottom edge — content passing behind the composer fades out instead of
  // being clipped by a hard line.
  const fadeMask = `linear-gradient(to bottom, #000 0, #000 calc(100% - ${String(bottomInset)}px), transparent 100%)`;

  // Only when there is genuinely nothing going on. A turn can be live (or can
  // have closed empty) while the transcript is still bare — a conversation
  // whose first turn died before any history row materialized reads as an
  // untouched chat, and this screen would bury both the dots and the error.
  if (!hasMessages && !turnHasRun) {
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
    <div
      ref={rootRef}
      className="relative flex min-h-0 flex-1 flex-col"
      data-empty-turn-detected={emptyTurnDetected || undefined}
    >
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
                                onBlockAction,
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
              streamingGroup &&
              !(turnPrinted ?? hasPrintableContent(group.messages));

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
                                onBlockAction,
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
                  <StreamingDots visible={awaitingFirstBlock} live />
                </div>
              </article>
            );
          })}
          {thinkingPresence.present ? (
            <ThinkingRow loading={thinking} overlay={streaming} />
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
