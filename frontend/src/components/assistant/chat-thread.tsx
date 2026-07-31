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
 */
const DOMINO_BALLS = 4;
const DOMINO_PERIOD_MS = 2_200;
// The exit sweep is a 280ms flight plus 72ms of stagger before the last ball
// leaves. 352ms is the floor; the margin here is deliberate slack so the final
// frame is painted rather than raced against the unmount.
const DOMINO_EXIT_MS = 380;
// Mirrors `.assistant-streaming-dot` in app.css: the shared negative delay
// starts every ball 24% into the cradle timeline, on the frame the right ball
// launches and they are all still level. The impulse then travels rightward
// until the right apex at 50% and leftward for the rest of the period. The two
// apex dwells have no instantaneous direction; each falls inside the pass that
// put the ball out there, so the sweep continues the travel already on screen.
const DOMINO_CSS_HEAD_START = 24 / 100;
const DOMINO_CSS_REVERSAL_KEYFRAME = 50 / 100;
// Elapsed time is measured from mount, which the head start has already
// consumed — so in that frame the rightward pass is split in two: it runs from
// mount to the reversal, and resumes once the timeline wraps past 100%.
const DOMINO_RIGHTWARD_UNTIL_MS =
  DOMINO_PERIOD_MS *
  (DOMINO_CSS_REVERSAL_KEYFRAME - DOMINO_CSS_HEAD_START);
const DOMINO_RIGHTWARD_FROM_MS =
  DOMINO_PERIOD_MS * (1 - DOMINO_CSS_HEAD_START);

/**
 * Horizontal offset out of a computed `transform`. Chromium resolves the
 * cradle's `translate3d` to a `matrix3d`, other engines may flatten it to the
 * 2D `matrix`, and jsdom reports nothing at all — hence the DOMMatrix path
 * with a hand parse behind it and a resting 0 behind that.
 */
function readTranslateX(transform: string): number {
  if (!transform || transform === "none") return 0;
  if (typeof DOMMatrixReadOnly === "function") {
    try {
      return new DOMMatrixReadOnly(transform).m41;
    } catch {
      // Fall through to the textual parse below.
    }
  }
  const values = /matrix(?:3d)?\(([^)]+)\)/.exec(transform)?.[1];
  if (!values) return 0;
  const parts = values.split(",").map(Number);
  // `matrix()` carries tx in slot 5; `matrix3d()` in slot 13.
  const x = parts.length === 6 ? parts[4] : parts[12];
  return typeof x === "number" && Number.isFinite(x) ? x : 0;
}

function StreamingDots({
  visible,
  live = false,
}: {
  readonly visible: boolean;
  readonly live?: boolean;
}) {
  const rootRef = useRef<HTMLSpanElement>(null);
  const mountedAt = useRef(0);
  const previousVisible = useRef(false);
  const lastVisibleRect = useRef<DOMRect | undefined>(undefined);
  const [present, setPresent] = useState(visible);

  if (visible && !present) setPresent(true);

  useLayoutEffect(() => {
    if (visible && !previousVisible.current) mountedAt.current = Date.now();
    previousVisible.current = visible;
  }, [visible]);

  useLayoutEffect(() => {
    if (visible && rootRef.current) {
      lastVisibleRect.current = rootRef.current.getBoundingClientRect();
    }
  });

  useLayoutEffect(() => {
    const root = rootRef.current;
    if (visible || !present || !root) return;

    const animatedDot = root.querySelector<HTMLElement>(
      ".assistant-streaming-dot",
    );
    const animation = animatedDot?.getAnimations?.()[0];
    const animationTime = animation?.currentTime;
    const elapsed =
      typeof animationTime === "number"
        ? animationTime
        : Date.now() - mountedAt.current;
    const phase =
      ((elapsed % DOMINO_PERIOD_MS) + DOMINO_PERIOD_MS) % DOMINO_PERIOD_MS;
    const rect = lastVisibleRect.current ?? root.getBoundingClientRect();

    // Hand each ball its own live position and brightness before the exit
    // animation replaces the cradle loop. Without this the exit interpolates
    // from the element's *underlying* style — no transform, resting opacity —
    // so a striker caught at its apex snaps a full 7px back into the pack on
    // the first frame and only then flies out. One frame of teleport undoes
    // every other claim this loader makes about mass.
    for (const ball of root.querySelectorAll<HTMLElement>(
      ".assistant-streaming-dot",
    )) {
      const computed = window.getComputedStyle(ball);
      ball.style.setProperty(
        "--domino-exit-x",
        `${String(readTranslateX(computed.transform))}px`,
      );
      if (computed.opacity) {
        ball.style.setProperty("--domino-exit-opacity", computed.opacity);
      }
    }

    root.dataset.exitDirection =
      phase < DOMINO_RIGHTWARD_UNTIL_MS || phase >= DOMINO_RIGHTWARD_FROM_MS
        ? "right"
        : "left";
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
    }, DOMINO_EXIT_MS);
    return () => {
      window.clearTimeout(exitTimer);
      delete root.dataset.exitDirection;
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
      for (const ball of root.querySelectorAll<HTMLElement>(
        ".assistant-streaming-dot",
      )) {
        ball.style.removeProperty("--domino-exit-x");
        ball.style.removeProperty("--domino-exit-opacity");
      }
    };
  }, [present, visible]);

  if (!present) return null;

  return (
    <span
      ref={rootRef}
      data-streaming-dots
      // 6px balls on an 8px pitch — a 30px pack, narrower than the five 4px
      // dots' 36px, with a 2px seam where they had a full 4px ball-width of
      // air between neighbours. That tightness is what lets a swing look like
      // a ball leaving the pack rather than a dot drifting between its
      // neighbours.
      className="assistant-streaming-dots relative flex h-[18px] w-max items-center gap-[2px]"
      // The standalone thinking row is itself a live region, so its dots stay
      // decorative. Dots inside an opened-but-still-empty assistant message
      // have no such wrapper — without their own role that whole pre-content
      // state is silent to a screen reader.
      role={visible && live ? "status" : undefined}
      aria-label={visible && live ? "Assistant is answering" : undefined}
      aria-hidden={!visible || !live ? true : undefined}
    >
      {Array.from({ length: DOMINO_BALLS }, (_, index) => (
        <span
          // The balls have no independent meaning; the parent owns the status.
          aria-hidden="true"
          // Opacity is animated (it tracks each ball's kinetic energy), so the
          // resting value lives in app.css beside the keyframes rather than as
          // a utility here that the animation would only override.
          className="assistant-streaming-dot h-[6px] w-[6px] rounded-full bg-muted-foreground"
          key={index}
        />
      ))}
    </span>
  );
}

/**
 * A turn that closed having printed nothing. The stream is over, so neither
 * the halo nor the dots are still honest, and an empty gutter reads as the
 * chat having died silently.
 */
function EmptyTurnError() {
  return (
    <p
      role="alert"
      data-empty-turn-error
      className="flex items-start gap-1.5 text-[13px] text-destructive"
    >
      <AlertCircle className="mt-[3px] h-3.5 w-3.5 shrink-0" />
      <span>Sorry, there seems to be an error with the request for now.</span>
    </p>
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
  // flash an error onto a turn that did answer.
  const tail = groups.at(-1);
  // `turnPrinted` when the pump can tell us; the transcript only as the
  // after-a-reload fallback, where it is the sole record that exists.
  const tailAnswered =
    turnPrinted ??
    (tail?.role === "assistant" && hasPrintableContent(tail.messages));
  const showEmptyTurnError = useSettled(
    turnEnded &&
      !thinking &&
      !streaming &&
      !tailAnswered &&
      // A read still in flight is the case the grace period is guessing at.
      // When the caller can tell us outright, don't guess: an answer that is
      // simply slow to project must never be reported as an error.
      !transcriptSettling,
    EMPTY_TURN_GRACE_MS,
  );

  // Deliberately the RAW terminal state, not the settled error. A turn that has
  // closed is never "no conversation yet", and gating the screen on the delayed
  // error would show "start a new conversation" all through the grace period —
  // or forever, if something keeps the settle condition suppressed.
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
                  {/* This group IS the tail, so its own answer never arrived. */}
                  {isLastGroup && showEmptyTurnError ? <EmptyTurnError /> : null}
                </div>
              </article>
            );
          })}
          {thinkingPresence.present ? (
            <ThinkingRow loading={thinking} overlay={streaming} />
          ) : null}
          {/* The turn closed before it ever spoke, so there is no assistant
              group to carry the message — give it its own row under a mark. */}
          {showEmptyTurnError && tail?.role !== "assistant" ? (
            <article className={ASSISTANT_ROW}>
              <AssistantIdentity time="" />
              <div className="min-w-0 flex-1">
                <EmptyTurnError />
              </div>
            </article>
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
